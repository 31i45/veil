//! 交互策略：按键到意图的翻译、编辑后的几何刷新编排、光标移动/命中/点选/
//! 拖选、剪贴板、菜单动作。只发意图与调几何，不做像素绘制。

use crate::app::{App, ConfigSave};
use crate::chrome::{self, MenuAction};
use crate::document::block_index::{gap_after, gap_between, BlockGeom};
use crate::editor::Intent;
use crate::geometry::{Rect, BASE_FONT_SIZE, BASE_LINE_HEIGHT, MARGIN, READABLE_COLS, WIDE_WINDOW};
use crate::html::html_to_markdown;
use crate::i18n::{t, tf, Key as I18nKey};
use crate::input::{Action, Key, Mods};
use crate::mode::{route, Cmd, Mode, TextSink};
use crate::paint::{
    active_cell_index, table_cell_start, table_delete_column, table_delete_row,
    table_insert_column, table_insert_row, table_set_align,
};
use crate::theme::{Config, Theme, ThemeMode};
use crate::view::ctx_menu::{CtxAction, CtxKind, TableCtx};
use crate::view::{ctx_menu, history, inline_details_spans, outline, quick_open, search};
use std::path::Path;
use std::time::{Duration, Instant};
use veil_core::{
    heading_slug, heading_slugs, task_span, BlockContainer, BlockId, BlockKind, CellAlign, HtmlTag,
    InlineKind, InlineNode, Range,
};
use winit::event::ElementState;
use winit::keyboard::{Key as WKey, KeyCode, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

/// 连续点击（双击/三击）的最大间隔。
const CLICK_GAP: Duration = Duration::from_millis(500);

/// 连续点击允许的位置漂移（逻辑像素）。
const CLICK_SLOP: f64 = 4.0;

/// 用户缩放一档对应的滚轮/捏合像素量（3c-2）：LineDelta 一格折算此值，
/// 高精度触控板（PixelDelta，Windows 捏合手势）按累积量满一档触发一次步进。
pub(crate) const ZOOM_WHEEL_PX: f32 = 50.0;
/// 用户缩放步进与量化的最小单位：加减法保证 ±档可精确往返 1.0。
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.5;
const ZOOM_MAX: f32 = 3.0;

impl App {
    /// 编辑后统一入口：重建索引、视口跟随光标、请求重绘。
    /// 仅内容编辑重启自动保存静默期；纯光标移动不推迟落盘。
    pub(crate) fn after_edit(&mut self, dirty: Vec<BlockId>) {
        if !dirty.is_empty() {
            self.persist_state.persist.note_activity(Instant::now());
        }
        // 脏块重解析时 BlockId 新鲜分配，派生缓存按 id 即自动失配；
        // 旧 id 条目在 refresh_index 的 prune 中随块真值回收
        // 先按新块结构重建索引，ensure_* 才能只遍历索引（不再各自全量枚举）
        self.refresh_index();
        if self.ui_state.outline.open {
            self.refresh_outline();
        }
        // 视口驱动：只排可见块；光标所在块可能已滚出视口，单独补齐
        self.ensure_layouts_visible();
        let caret_id = self.caret_block_index().map(|i| self.index.blocks[i].id);
        if let Some(id) = caret_id {
            self.ensure_block(id);
        }
        self.ensure_tables_visible();
        // 布局/表格已就绪：高度来源稳定后才累计 top
        self.recompute_geometry();
        self.scroll_to_caret();
        self.update_title(); // 脏标随编辑翻转

        // P0 探针：仅在确有脏块（真实编辑）时记录一次快照；关闭时零开销。
        if !dirty.is_empty() {
            let session = self.editor.session();
            crate::perf::edit(crate::perf::EditSnapshot {
                dirty_blocks: dirty.len(),
                total_blocks: session.block_count(),
                rescan_lines: session.last_rescan_lines(),
                undo_depth: session.undo_depth(),
            });
        }
        self.request_redraw();
    }

    pub(crate) fn on_key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        // 任意按下都算活跃：Ctrl/Shift 等不产生 Action、也不移动 caret 的键
        // 同样要让隐藏半周期里的光标立刻回来。
        self.note_caret_activity();
        let started = Instant::now();
        let Some(key) = map_key(event, self.input_state.mods) else {
            return;
        };
        self.route_key(key);
        crate::perf::key(started.elapsed());
    }

    /// 单个内部键的路由与执行：`on_key` 与测试共用同一入口。
    /// 模式优先级与各模式的按键语义只在 `mode::route` 一处表达。
    pub(crate) fn route_key(&mut self, key: Key) {
        // 设置模态优先（优先级高于普通 modal）：打开时它是唯一输入接收者
        if self.ui_state.settings.is_some() {
            let mods = self.input_state.mods;
            // 先更新 layout（scale 变化时需要）
            let layout = self.settings_layout();
            let id = if let Some(s) = self.ui_state.settings.as_mut() {
                let id = s.state.on_key(&key, mods, &mut s.form, &layout);
                // 低高度滚动：键盘焦点移到视口外时自动滚到可见（DEF-T07-E1-001）
                s.state.reveal_focused(layout.scroll, layout.shown_rows);
                id
            } else {
                None
            };
            match id {
                Some(id) => self.resolve_settings(id),
                None => self.request_redraw(),
            }
            return;
        }
        // 模态优先：打开时它是唯一输入接收者，其余路由（菜单/浮层/正文）全部让位
        if self.ui_state.modal.is_some() {
            let shift = self.input_state.mods.shift;
            let id = if let Some(a) = self.ui_state.modal.as_mut() {
                a.state.on_key(&key, shift, &a.spec)
            } else {
                None
            };
            match id {
                Some(id) => self.resolve_modal(id),
                None => self.request_redraw(),
            }
            return;
        }
        let cmd = route(
            self.mode(),
            key,
            self.input_state.mods,
            self.editor.preedit().is_some(),
        );
        match cmd {
            Cmd::Ignore => {}
            Cmd::OpenQuick => self.toggle_quick(),
            Cmd::CloseMenu => self.close_top_panel(),
            Cmd::MenuMove(d) => self.menu_move(d),
            Cmd::MenuSwitch(d) => self.menu_switch(d),
            Cmd::MenuAccept => self.menu_accept(),
            // 快速打开浮层动作自带重绘与 IME 区域刷新
            Cmd::QuickClose => self.close_quick(),
            Cmd::QuickAccept => self.quick_accept(),
            Cmd::QuickMove(d) => self.quick_step(d),
            Cmd::QuickFirst => self.quick_first(),
            Cmd::QuickLast => self.quick_last(),
            Cmd::QuickPage(d) => self.quick_page(d),
            Cmd::QuickBackspace => self.quick_backspace(),
            Cmd::QuickInput(t) => self.quick_input(&t),
            // 查找栏动作自带重绘与 IME 区域刷新
            Cmd::CloseSearch => self.close_search(),
            Cmd::SearchInput(t) => self.search_input(&t),
            Cmd::SearchBackspace => self.search_backspace(),
            Cmd::SearchPaste => self.search_paste(),
            Cmd::SearchToggleField => self.search_toggle_field(),
            Cmd::SearchNext => self.search_enter(),
            Cmd::SearchPrev => self.search_prev(),
            Cmd::SearchFirstMatch => self.search_first_match(),
            Cmd::SearchLastMatch => self.search_last_match(),
            // 版本历史：导航/恢复/关闭（预览只读，动作自带重绘）
            Cmd::HistoryClose => self.close_history(),
            Cmd::HistoryMove(d) => self.history_step(d),
            Cmd::HistoryRestore => self.history_restore_selected(),
            Cmd::Edit(Action::Doc(Intent::Indent)) if !self.editor.preedit().is_some() => {
                if !self.tab_next_cell() {
                    self.perform_action(Action::Doc(Intent::Indent));
                }
                self.set_ime_area();
            }
            Cmd::Edit(Action::Doc(Intent::Dedent)) if !self.editor.preedit().is_some() => {
                if !self.tab_prev_cell() {
                    self.perform_action(Action::Doc(Intent::Dedent));
                }
                self.set_ime_area();
            }
            Cmd::Edit(Action::Doc(Intent::Newline)) if self.input_state.mods.ctrl => {
                if !self.table_ctrl_enter() {
                    self.perform_action(Action::Doc(Intent::Newline));
                }
                self.set_ime_area();
            }
            Cmd::Edit(action) => {
                self.perform_action(action);
                self.set_ime_area();
            }
        }
    }

    /// IME 组合串：唯一存储在 `Editor`，落点由 `TextSink` 决定绘制位置
    /// （文档行内 / 查找栏）。组合期按键由 `route` 让给 IME。
    pub(crate) fn ime_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if self.ui_state.modal.is_some() || self.mode() == Mode::History {
            return; // 模态独占输入；只读预览不显示组合串
        }
        self.editor.set_preedit(text, cursor);
        self.request_redraw();
    }

    /// IME 提交：按落点分流——模态转键盘助记符，菜单吞掉，查找栏进查询串，否则进文档。
    pub(crate) fn ime_commit(&mut self, text: String) {
        // Windows 下普通字母也可能经 IME Commit 到达（例如中文输入法处于英文模式）。
        // 必须复用模态键盘路由，不能绕过它直接写正文。
        if self.ui_state.modal.is_some() {
            self.route_key(Key::Text(text));
            return;
        }
        match self.text_sink() {
            TextSink::Quick => {
                self.editor.clear_preedit();
                self.quick_input(&text);
            }
            TextSink::Query => {
                self.editor.clear_preedit();
                self.search_input(&text);
            }
            TextSink::Doc => {
                if matches!(self.mode(), Mode::Menu | Mode::History) {
                    return; // 菜单/版本历史：吞掉输入，预览保持只读
                }
                // IME 提交同样属于输入活跃：候选框落字后光标不应继续藏着。
                self.note_caret_activity();
                let dirty = self.editor.commit_ime(text);
                self.after_edit(dirty);
                self.set_ime_area();
            }
        }
    }

    pub(crate) fn ime_disabled(&mut self) {
        self.editor.clear_preedit();
    }

    /// 文档级动作执行：文档意图、视图移动、剪贴板、文件与菜单动作的唯一分支点。
    /// 入口统一关闭 chrome 菜单与右键菜单——任何 app 级动作都不应在模态覆盖层下执行。
    /// （点击路径先于 perform_action 手动 close，这里是兜底幂等，不冲突）
    pub(crate) fn perform_action(&mut self, action: Action) {
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        match action {
            Action::Doc(intent) => {
                self.window_state.goal_x = None; // 非垂直移动：重置目标列
                                                 // 行移动只置换行内容：块的行区间与块 id 均不变，而布局键不含内容指纹，
                                                 // 不换会话代次会命中旧布局（画面滞后到下一次编辑才刷新）。
                let line_move = matches!(intent, Intent::MoveLineUp | Intent::MoveLineDown);
                if let Some(dirty) = self.editor.apply(intent) {
                    if line_move {
                        self.input_state.revs.session += 1;
                    }
                    self.after_edit(dirty);
                }
            }
            // 垂直移动/扩选：保留 goal_x（连续上下不漂移）
            Action::MoveVertical(delta) => self.move_vertical(delta, false),
            Action::Page(dir) => self.page_vertical(dir),
            Action::SelectVertical(delta) => self.move_vertical(delta, true),
            Action::MoveLineStart => self.move_line_edge(true, false),
            Action::MoveLineEnd => self.move_line_edge(false, false),
            Action::SelectLineStart => self.move_line_edge(true, true),
            Action::SelectLineEnd => self.move_line_edge(false, true),
            Action::Copy => self.copy_selection(),
            Action::Cut => self.cut_selection(),
            Action::Paste => self.paste_clipboard(),
            Action::Save => self.save_now(),
            Action::SaveAs => self.save_as(),
            Action::Open => self.open_via_dialog(),
            Action::New => self.new_draft(),
            Action::NewWindow => self.window_state.new_window_requested = true,
            Action::QuickOpen => self.toggle_quick(),
            Action::CloseMenu => self.close_top_panel(),
            Action::OpenOutline => self.toggle_outline(),
            Action::OpenSourceMode => self.toggle_source_mode(),
            Action::Export => self.export_now(),
            Action::ExportPdf => self.export_pdf(),
            Action::Print => self.print_document(),
            Action::VersionHistory => self.toggle_history(),
            Action::Find => self.toggle_search(),
            Action::Replace => self.toggle_replace(),
            Action::InsertImage => self.insert_image_via_dialog(),
            Action::ZoomIn => self.zoom_step(1),
            Action::ZoomOut => self.zoom_step(-1),
            Action::ZoomReset => self.zoom_reset(),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::OpenSettings => self.open_settings(),
        }
    }

    /// 全屏切换（3c-3）：F11 / 视图菜单；无边框全屏（Borderless，保留任务栏快速切出）。
    /// Esc 不退出全屏（Esc 只关菜单——避免与「关掉手上面板」的肌肉记忆冲突）。
    /// 窗口尺寸随系统 Resized 事件变化，重排/滚动钳位走既有路径，此处只翻转状态。
    pub(crate) fn toggle_fullscreen(&mut self) {
        let Some(w) = &self.window_state.window else {
            return; // headless / 窗口未创建：无操作
        };
        let on = !self.window_state.fullscreen;
        w.set_fullscreen(on.then_some(winit::window::Fullscreen::Borderless(None)));
        self.window_state.fullscreen = on;
        self.ui_state.chrome.notice = if on {
            t(I18nKey::NoticeFullscreenOn)
        } else {
            t(I18nKey::NoticeFullscreenOff)
        }
        .into();
        self.request_redraw();
    }

    // ── 用户缩放（3c-2）：effective scale = DPI × user_zoom ──────────────────

    /// 滚轮/捏合增量入口（像素）：累积满一档阈值触发一次步进，余量保留
    /// （高精度触控板一次捏合是数十个小增量事件，逐事件步进会过度敏感）。
    pub(crate) fn zoom_by(&mut self, px: f32) {
        self.input_state.zoom_accum += px;
        while self.input_state.zoom_accum >= ZOOM_WHEEL_PX {
            self.input_state.zoom_accum -= ZOOM_WHEEL_PX;
            self.zoom_step(1);
        }
        while self.input_state.zoom_accum <= -ZOOM_WHEEL_PX {
            self.input_state.zoom_accum += ZOOM_WHEEL_PX;
            self.zoom_step(-1);
        }
    }

    /// 缩放一档（+1 放大 / -1 缩小）。
    pub(crate) fn zoom_step(&mut self, dir: i32) {
        let z = self.input_state.user_zoom + ZOOM_STEP * dir as f32;
        self.set_user_zoom(z);
    }

    /// 还原 100%（Ctrl+0 / 视图菜单）。
    pub(crate) fn zoom_reset(&mut self) {
        self.set_user_zoom(1.0);
    }

    /// 设置用户缩放因子：0.1 量化 + [0.5, 3.0] 钳位；有效缩放 = DPI × 用户因子。
    /// base_key 含 scale 量化值 → 布局/光栅/表格整体失配自然重排；
    /// chrome 标签按旧 scale 光栅化，与 ScaleFactorChanged 同路显式作废。
    pub(crate) fn set_user_zoom(&mut self, zoom: f32) {
        if self.window_state.print.is_some() {
            return; // 打印页帧语义独占 scale（模态同步流程，防御性拒绝）
        }
        let zoom = ((zoom * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX);
        if zoom == self.input_state.user_zoom {
            return;
        }
        self.input_state.user_zoom = zoom;
        self.input_state.scale = self.input_state.dpi_scale * zoom;
        self.ui_state.chrome.clear_label_cache();
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        // 文档高度随缩放变化：滚动位置可能越出新的最大滚动量，钳位
        let max = chrome::max_scroll(self.viewport(), self.doc_height());
        self.window_state.goal_y = self.window_state.goal_y.clamp(0.0, max);
        if !self.window_state.smooth {
            self.window_state.scroll_y = self.window_state.goal_y;
        }
        self.set_ime_area();
        self.ui_state.chrome.notice = tf(
            I18nKey::NoticeZoom,
            &[&((zoom * 100.0).round() as u32).to_string()],
        );
        self.request_redraw();
    }

    /// 粘贴（切片 11/14）：文件列表中的图片优先（资源管理器 Ctrl+C），
    /// 其次 HTML→Markdown，再纯文本；文本皆空时尝试位图：落盘 `<doc>.assets/` 并插入链接。
    /// 草稿（无关联文件）拒绝图片粘贴（无写入基准），文档不变、不置脏。
    pub(crate) fn paste_clipboard(&mut self) {
        // 文件复制在 Windows 上同时附带路径文本，故文件判定必须先于文本分支，
        // 否则粘出来的是路径字符串而不是图片。非图片文件不在此处理，仍走文本分支。
        let images: Vec<_> = self
            .ports
            .clipboard
            .get_files()
            .into_iter()
            .filter(|p| crate::image::is_image_path(p))
            .collect();
        if !images.is_empty() {
            self.insert_image_files(&images);
            return;
        }
        let md = self
            .ports
            .clipboard
            .get_html()
            .map(|h| {
                let mut md = html_to_markdown(&h);
                // 转换结果恒以单个换行收尾；原样插入会在行中凭空断行（B7-3），剥掉这一个
                if md.ends_with('\n') {
                    md.pop();
                }
                md
            })
            .filter(|m| !m.trim().is_empty());
        if let Some(text) = md.or_else(|| self.ports.clipboard.get()) {
            self.insert_paste(&text);
            return;
        }
        let Some(img) = self.ports.clipboard.get_image() else {
            return;
        };
        self.insert_image_data(&img);
    }

    /// 位图插入（切片 14 / 阶段 1 拖拽共用）：落盘 `<doc>.assets/` 并插入 `![](相对路径)`。
    /// 草稿（无关联文件）拒绝（无写入基准），文档不变、不置脏；落盘失败同样保持原样并提示。
    pub(crate) fn insert_image_data(&mut self, img: &crate::image::ImageData) {
        let Some(doc) = self.doc.path.clone() else {
            self.ui_state.chrome.notice = t(I18nKey::NoticeImagePasteNeedsFile).into();
            eprintln!("图片粘贴需要关联文件（草稿无落盘目录）");
            self.request_redraw();
            return;
        };
        match self.ports.assets.save_pasted_image(&doc, img) {
            Ok(rel) => self.insert_paste(&self.image_paragraph_markup(&rel)),
            Err(e) => {
                self.ui_state.chrome.notice = tf(I18nKey::NoticeImageSaveFailed, &[&e.to_string()]);
                eprintln!("图片落盘失败（文档未改动）: {e}");
                self.request_redraw();
            }
        }
    }

    /// 图片文件插入（剪贴板文件列表）：与拖入同一条路径——解码 → 落盘 assets → 独占段插入。
    /// 多个图片文件按顺序各成一段；草稿/落盘失败语义由 `insert_image_data` 统一处理。
    fn insert_image_files(&mut self, paths: &[std::path::PathBuf]) {
        for path in paths {
            match self.ports.images.load(path) {
                Ok(img) => self.insert_image_data(&img),
                Err(e) => {
                    self.ui_state.chrome.notice =
                        tf(I18nKey::NoticeImageReadFailed, &[&e.to_string()]);
                    eprintln!("图片读取失败（文档未改动）{}: {e}", path.display());
                    self.request_redraw();
                }
            }
        }
    }

    /// 插入图片入口（2f）：菜单「插入图片」→ 系统对话框选图 → 插入相对路径链接。
    /// 草稿（无关联文件）拒绝（无相对路径基准）；非图片扩展名拒绝；取消无副作用。
    pub(crate) fn insert_image_via_dialog(&mut self) {
        let Some(doc) = self.doc.path.clone() else {
            self.ui_state.chrome.notice = t(I18nKey::NoticeInsertImageNeedsFile).into();
            self.request_redraw();
            return;
        };
        let Some(img) = self.ports.dialogs.open_image() else {
            return;
        };
        if !crate::image::is_image_path(&img) {
            self.ui_state.chrome.notice = t(I18nKey::NoticeUnsupportedImage).into();
            self.request_redraw();
            return;
        }
        let target = image_link_target(&doc, &img);
        self.insert_paste(&self.image_paragraph_markup(&target));
    }

    /// 拖拽悬停（阶段 1）：记录文件名供绘制落点提示；离开窗口（None）清空。
    pub(crate) fn hover_file(&mut self, path: Option<std::path::PathBuf>) {
        self.input_state.drop_file = path;
        self.request_redraw();
    }

    /// 拖放文件（阶段 1）：图片解码后插入落点；其余文件按文档打开。
    /// 落点 = 拖拽期间的鼠标位置（未命中正文时沿用当前光标）。
    pub(crate) fn drop_file(&mut self, path: std::path::PathBuf) {
        self.input_state.drop_file = None;
        if self.ui_state.modal.is_some() {
            return; // 模态独占：拖放不改变文档，也不叠加打开意图
        }
        // 拖放是文档级动作：先收起浮层/菜单，避免编辑与浮层共存
        self.close_quick();
        self.close_search();
        self.ui_state.chrome.close();
        if !crate::image::is_image_path(&path) {
            self.open_path(path);
            return;
        }
        self.point_caret(false); // 图片插入点 = 鼠标落点
        match self.ports.images.load(&path) {
            Ok(img) => self.insert_image_data(&img),
            Err(e) => {
                self.ui_state.chrome.notice = tf(I18nKey::NoticeImageReadFailed, &[&e.to_string()]);
                eprintln!("图片读取失败（文档未改动）{}: {e}", path.display());
                self.request_redraw();
            }
        }
    }

    /// 粘贴文本/图片链接：有选区则替换，单次撤销事务；空串不产生编辑。
    pub(crate) fn insert_paste(&mut self, text: &str) {
        if let Some(dirty) = self.editor.paste(text) {
            self.window_state.goal_x = None;
            self.after_edit(dirty);
        }
    }

    /// Typora 式「图片独占一段」：根据光标/选区起点与周围文本的关系，
    /// 构建 `![](path)` 前后的换行，让图片 markdown 自成段落。
    /// - 光标在行首/文档头：前缀空
    /// - 光标在行尾/文档尾：后缀空
    /// - 有选区：以选区起点为插入基准（选区整体被替换）
    fn image_paragraph_markup(&self, path: &str) -> String {
        let session = self.editor.session();
        let text_len = session.text_len();
        let caret = self.editor.caret();
        let start = self.editor.selection().map(|s| s.start).unwrap_or(caret);

        // 行首判定：光标前一字节是 \n，或就在文档开头
        let at_line_start = start == 0
            || session
                .text_slice(Range::new(start - 1, start))
                .map(|s| s == "\n")
                .unwrap_or(false);
        let prefix = if at_line_start { "" } else { "\n" };

        // 行尾判定：光标后一字节是 \n，或就在文档末尾
        let at_line_end = start >= text_len
            || session
                .text_slice(Range::new(start, (start + 1).min(text_len)))
                .map(|s| s == "\n")
                .unwrap_or(false);
        let suffix = if at_line_end { "" } else { "\n" };

        format!("{prefix}![]({path}){suffix}")
    }

    /// Home/End（及 Shift 扩选）：光标落到当前视觉行首/尾（`visual_line_edges` 语义）。
    pub(crate) fn move_line_edge(&mut self, start: bool, extend: bool) {
        let caret = self.editor.caret();
        let Some((id, range)) = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= caret && caret <= g.range.end)
            .map(|g| (g.id, g.range))
        else {
            return;
        };
        self.ensure_block(id);
        let Some(l) = self.cache.layouts.get(&id) else {
            return;
        };
        let (s, e) = self.engine.visual_line_edges(l, caret - range.start);
        let target = range.start + if start { s } else { e };
        let moved = if extend {
            self.editor.set_caret_extend(target)
        } else {
            self.editor.set_caret(target)
        };
        if !moved {
            return;
        }
        self.window_state.goal_x = None; // 水平移动：重置目标列
        self.request_redraw();
    }

    pub(crate) fn move_vertical(&mut self, delta: i32, extend: bool) {
        let caret = self.editor.caret();
        let Some(bi) =
            self.index.blocks.iter().position(|g| {
                !self.block_hidden(g) && g.range.start <= caret && caret <= g.range.end
            })
        else {
            return;
        };
        let (id, range) = {
            let g = &self.index.blocks[bi];
            (g.id, g.range)
        };
        // 当前块可能不在视口内：垂直移动前按需补齐
        self.ensure_block(id);
        let (cx, cy, ch, lh) = {
            let Some(l) = self.cache.layouts.get(&id) else {
                return;
            };
            let (cx, cy, ch) = self.engine.caret_xy(l, caret - range.start);
            (cx, cy, ch, l.height)
        };
        let goal = *self.window_state.goal_x.get_or_insert(cx);
        // 目标：本块上/下一视觉行；越界则上/下一块的末/首行
        let target_y_local = cy + if delta < 0 { -1.0 } else { ch };
        let target = if target_y_local >= 0.0 && target_y_local < lh {
            let l = &self.cache.layouts[&id];
            range.start + self.engine.hit(l, goal, target_y_local)
        } else {
            // B3：折叠 details 的隐藏子块不接收光标——上下移动跳过整组。
            let next_i = if delta < 0 {
                (0..bi)
                    .rev()
                    .find(|&i| !self.block_hidden(&self.index.blocks[i]))
            } else {
                (bi + 1..self.index.blocks.len())
                    .find(|&i| !self.block_hidden(&self.index.blocks[i]))
            };
            let Some(ni) = next_i else {
                return;
            };
            let (nid, nkind, nrange) = {
                let g = &self.index.blocks[ni];
                (g.id, g.kind.clone(), g.range)
            };
            // 目标相邻块同样按需补齐（含表格网格/数学图）
            self.ensure_block(nid);
            if nkind == BlockKind::Table && self.table_shows_grid(nid, &nrange) {
                let Some(t) = self.cache.tables.get(&nid) else {
                    return;
                };
                t.hit(goal, if delta < 0 { t.height - 1.0 } else { 0.0 })
            } else {
                let Some(nl) = self.cache.layouts.get(&nid) else {
                    return;
                };
                let y = if delta < 0 { nl.height - 1.0 } else { 0.0 };
                nrange.start + self.engine.hit(nl, goal, y).min(nrange.len())
            }
        };
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.scroll_to_caret();
        self.request_redraw();
    }

    /// PageUp/PageDown：按视口翻页（滚动量复用 `chrome::page_scroll`），光标随页移动，
    /// 保持原列（goal_x）并夹在新视口内。
    pub(crate) fn page_vertical(&mut self, dir: i32) {
        if dir == 0 {
            return;
        }
        let vp = self.viewport();
        let old_scroll = self.window_state.scroll_y;
        let new_scroll = chrome::page_scroll(vp, self.doc_height(), old_scroll, dir.signum());
        let caret = self.editor.caret();
        let Some(bi) = self.caret_block_index() else {
            self.window_state.goal_y = new_scroll;
            if !self.window_state.smooth {
                self.window_state.scroll_y = self.window_state.goal_y;
            }
            self.request_redraw();
            return;
        };
        let (id, range, top) = {
            let g = &self.index.blocks[bi];
            (g.id, g.range, g.top)
        };
        self.ensure_block(id);
        let (cx, cy, ch) = {
            let Some(l) = self.cache.layouts.get(&id) else {
                return;
            };
            self.engine.caret_xy(l, caret - range.start)
        };
        let goal = *self.window_state.goal_x.get_or_insert(cx);
        let line_h = ch.max(1.0);
        let old_bottom = (old_scroll + vp.h - line_h).max(old_scroll);
        let base = (top + cy).clamp(old_scroll, old_bottom);
        let target_y = (base + dir.signum() as f32 * vp.h * 0.9)
            .clamp(new_scroll, (new_scroll + vp.h - line_h).max(new_scroll));
        // 目标行所在块（块间空白归入上一块，与 hit_offset 同口径）
        let ti = self
            .index
            .blocks
            .iter()
            .rposition(|g| g.top <= target_y)
            .unwrap_or(0);
        let (tid, tkind, trange, ttop) = {
            let g = &self.index.blocks[ti];
            (g.id, g.kind.clone(), g.range, g.top)
        };
        self.ensure_block(tid);
        let target = if tkind == BlockKind::Table && self.table_shows_grid(tid, &trange) {
            let Some(t) = self.cache.tables.get(&tid) else {
                return;
            };
            t.hit(goal, (target_y - ttop).clamp(0.0, t.height - 1.0))
        } else {
            let Some(l) = self.cache.layouts.get(&tid) else {
                return;
            };
            trange.start
                + self
                    .engine
                    .hit(l, goal, (target_y - ttop).max(0.0))
                    .min(trange.len())
        };
        self.window_state.goal_y = new_scroll;
        if !self.window_state.smooth {
            self.window_state.scroll_y = self.window_state.goal_y;
        }
        if !self.editor.set_caret(target) {
            return;
        }
        self.scroll_to_caret();
        self.request_redraw();
    }

    /// 块内物理坐标 → 文档偏移：表格网格态走单元格命中（落单元格内容起点），
    /// 其余走文本布局命中。
    pub(crate) fn hit_block(
        &self,
        id: BlockId,
        kind: BlockKind,
        range: &veil_core::Range,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        if kind == BlockKind::Table && self.table_shows_grid(id, range) {
            return self.cache.tables.get(&id).map(|t| t.hit(x, y));
        }
        let l = self.cache.layouts.get(&id)?;
        Some(range.start + self.engine.hit(l, x, y).min(range.len()))
    }

    /// 鼠标位置 → 文档偏移（块间空白归入下一块起始；视口顶部之上归首块）。
    pub(crate) fn hit_offset(&mut self) -> Option<usize> {
        let vp = self.viewport();
        let x = (self.input_state.mouse.x as f32 - self.content_x()).max(0.0);
        let y = (self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y).max(0.0);
        // 文末输入空间（B5）：点击末块下方的留白 → 光标直接落文档末尾，
        // 而不是被末块的行尾钳制吸到最后一行的某个字上。
        if let Some(last) = self
            .index
            .blocks
            .iter()
            .rev()
            .find(|g| !self.block_hidden(g))
        {
            if y >= last.top + self.block_height(last) {
                return Some(self.editor.session().text_len());
            }
        }
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let g = blocks
                .iter()
                .rev()
                .find(|g| !self.block_hidden(g) && g.top <= y)
                .or_else(|| blocks.iter().find(|g| !self.block_hidden(g)))?;
            (g.id, g.kind.clone(), g.range, g.top)
        };
        // 命中块可能尚未布局：先按需补齐再读缓存
        self.ensure_block(id);
        self.hit_block(id, kind, &range, x, (y - top).max(0.0))
    }

    /// 点击任务列表复选框（P3-5）：命中方框区域则切换勾选态并返回 true
    /// （光标保持原位）。方框区域 = 任务项首行、内容文本起点左侧
    /// （缩进起至尾随空格前）；激活态显源码时点击 `[ ]` 同样可切换。
    pub(crate) fn try_toggle_task(&mut self) -> bool {
        let vp = self.viewport();
        let x = self.input_state.mouse.x as f32 - self.content_x();
        let y = self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y;
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let Some(g) = blocks
                .iter()
                .rev()
                .find(|g| !self.block_hidden(g) && g.top <= y)
                .or_else(|| blocks.iter().find(|g| !self.block_hidden(g)))
            else {
                return false;
            };
            (g.id, g.kind.clone(), g.range, g.top)
        };
        let BlockKind::ListItem {
            task: Some(cur), ..
        } = kind
        else {
            return false;
        };
        self.ensure_block(id);
        let Some(l) = self.cache.layouts.get(&id) else {
            return false;
        };
        let Some(text) = self.editor.session().text_slice(range) else {
            return false;
        };
        let body = text.trim_end_matches(['\n', '\r']);
        let Some((start, state_pos, content_start, _)) = task_span(body) else {
            return false;
        };
        // 首行判定 + 方框区域：[缩进起 x, 文本起点 x)（☐ 与尾随空格均为命中区；
        // 替换段内部 doc 偏移钳到布局起点，故不得用尾随空格前偏移取右缘）
        let cy = y - top;
        let (tx, ty, th) = self.engine.caret_xy(l, content_start);
        if cy < ty || cy >= ty + th {
            return false;
        }
        let (ix, _, _) = self.engine.caret_xy(l, start);
        if x < ix || x >= tx {
            return false;
        }
        let state = veil_core::Range::new(range.start + state_pos, range.start + state_pos + 1);
        let Some(dirty) = self.editor.toggle_task(state, !cur) else {
            return false;
        };
        self.after_edit(dirty);
        true
    }

    /// 落点更新 caret。`extend` = 保留 anchor（拖选/Shift+点击）。
    pub(crate) fn point_caret(&mut self, extend: bool) {
        let Some(target) = self.hit_offset() else {
            return;
        };
        // 点击定位/拖选：即使落到同一位置（caret 不变）也要让光标恢复可见。
        self.note_caret_activity();
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.window_state.goal_x = None;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 最近文件菜单标签（0b）：文件名优先，无文件名回退全路径。
    /// 布局/命中/分发共用（chrome::menu_rows 的 recent 入参），保证三者一致。
    pub(crate) fn recent_labels(&self) -> Vec<String> {
        self.persist_state
            .recent
            .iter()
            // 与状态栏/标题/notice 同一套 `文件名 · 父目录` 标签：
            // 不同目录下的同名文件中可区分，也避免完整路径挤爆菜单宽度。
            .map(|p| crate::chrome::path_label(Some(p.as_path())))
            .collect()
    }

    /// 菜单开关项与可用性标志的唯一构造点：绘制与命中/执行共用，避免两处漂移。
    pub(crate) fn menu_flags(&self) -> chrome::MenuFlags {
        chrome::MenuFlags {
            fullscreen: self.window_state.fullscreen,
            focus: self.ui_state.writing.focus,
            typewriter: self.ui_state.writing.typewriter,
            source: self.source_mode(),
            block_remote_images: !self.config.allow_remote_images,
            can_save: self.editor.is_dirty() || self.doc.path.is_none(),
            can_undo: self.editor.session().undo_depth() > 0,
            can_redo: self.editor.session().redo_depth() > 0,
        }
    }

    /// 打开最近文件（0b）：复用 open_path（脏文档拒绝；下标越界静默忽略）。
    pub(crate) fn open_recent(&mut self, i: usize) {
        let Some(path) = self.persist_state.recent.get(i).cloned() else {
            return;
        };
        self.open_path(path);
    }

    /// 菜单键盘导航（C1）：移动选中项。右键菜单优先；无选中时 ↓ 选首项、↑ 选末项，
    /// 有选中时钳位在 `[0, n-1]`。高亮复用绘制既有的 `ctx.hover` / `chrome.hover_item`。
    pub(crate) fn menu_move(&mut self, d: i32) {
        if self.ui_state.ctx.open {
            // 可停靠项：跳过分隔线
            let nav: Vec<usize> = self
                .ui_state
                .ctx
                .items()
                .iter()
                .enumerate()
                .filter(|(_, (_, a))| *a != CtxAction::Separator)
                .map(|(i, _)| i)
                .collect();
            if nav.is_empty() {
                return;
            }
            let i = match self
                .ui_state
                .ctx
                .hover
                .and_then(|h| nav.iter().position(|&r| r == h))
            {
                Some(p) => (p as i32 + d).clamp(0, nav.len() as i32 - 1) as usize,
                None if d < 0 => nav.len() - 1,
                None => 0,
            };
            self.ui_state.ctx.hover = Some(nav[i]);
            self.request_redraw();
            return;
        }
        let Some(open) = self.ui_state.chrome.open else {
            return;
        };
        let recent = self.recent_labels();
        let rows = chrome::menu_rows(open, &recent);
        // 可停靠项：跳过分隔线与禁用项（二者无命中反馈、不可执行）
        let flags = self.menu_flags();
        let nav: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, _, a))| chrome::action_enabled(*a, flags))
            .map(|(i, _)| i)
            .collect();
        if nav.is_empty() {
            return;
        }
        // 只认当前菜单的选中项：切换菜单后残留的旧下标不得参与移动
        let current = self
            .ui_state
            .chrome
            .hover_item
            .filter(|(m, _)| *m == open)
            .and_then(|(_, i)| nav.iter().position(|&r| r == i));
        let i = match current {
            Some(p) => (p as i32 + d).clamp(0, nav.len() as i32 - 1) as usize,
            None if d < 0 => nav.len() - 1,
            None => 0,
        };
        self.ui_state.chrome.hover_item = Some((open, nav[i]));
        self.request_redraw();
    }

    /// 主菜单左右切换（Left/Right）：右键菜单打开时不切换主菜单。
    /// 循环取模，切换后清空旧菜单的选中项，只保留悬停高亮。
    pub(crate) fn menu_switch(&mut self, d: i32) {
        if self.ui_state.ctx.open {
            return;
        }
        let Some(open) = self.ui_state.chrome.open else {
            return;
        };
        let next = (open as i32 + d).rem_euclid(chrome::MENUS.len() as i32) as usize;
        self.ui_state.chrome.open = Some(next);
        self.ui_state.chrome.hover_item = None;
        self.ui_state.chrome.hover_menu = Some(next);
        self.request_redraw();
    }

    /// 菜单键盘执行（C1）：只执行有选中项的动作；无选中时不做任何事（不误编辑文档）。
    pub(crate) fn menu_accept(&mut self) {
        if self.ui_state.ctx.open {
            let Some(i) = self.ui_state.ctx.hover else {
                return;
            };
            if let Some(action) = self.ui_state.ctx.action(i) {
                self.ctx_exec(action);
            }
            self.ui_state.ctx.close();
            self.request_redraw();
            return;
        }
        let Some(open) = self.ui_state.chrome.open else {
            return;
        };
        let Some((menu, item)) = self.ui_state.chrome.hover_item else {
            return;
        };
        if menu != open {
            return;
        }
        let recent = self.recent_labels();
        let Some(action) = chrome::action_at(open, item, &recent) else {
            return;
        };
        if !chrome::action_enabled(action, self.menu_flags()) {
            return; // 禁用项：回车无效果，菜单保持打开
        }
        self.ui_state.chrome.close();
        self.request_redraw();
        self.perform_menu(action);
    }

    /// 执行菜单动作。返回 true = 退出应用（现由模态结论写 exit_requested，恒为 false）。
    pub(crate) fn perform_menu(&mut self, action: MenuAction) -> bool {
        match action {
            MenuAction::New => self.new_draft(),
            MenuAction::NewWindow => self.window_state.new_window_requested = true,
            MenuAction::Open => self.open_via_dialog(),
            MenuAction::Save => self.save_now(),
            MenuAction::SaveAs => self.save_as(),
            MenuAction::Export => self.export_now(),
            MenuAction::ExportPdf => self.export_pdf(),
            MenuAction::Print => self.print_document(),
            MenuAction::VersionHistory => self.toggle_history(),
            MenuAction::Undo => {
                if let Some(dirty) = self.editor.apply(Intent::Undo) {
                    self.window_state.goal_x = None;
                    self.after_edit(dirty);
                }
            }
            MenuAction::Redo => {
                if let Some(dirty) = self.editor.apply(Intent::Redo) {
                    self.window_state.goal_x = None;
                    self.after_edit(dirty);
                }
            }
            MenuAction::Cut => self.cut_selection(),
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Paste => self.paste_clipboard(),
            MenuAction::SelectAll => self.perform_action(Action::Doc(Intent::SelectAll)),
            MenuAction::ThemeSystem => self.set_theme_mode(ThemeMode::System),
            MenuAction::ThemeLight => self.set_theme_mode(ThemeMode::Light),
            MenuAction::ThemeDark => self.set_theme_mode(ThemeMode::Dark),
            MenuAction::ToggleFocus => self.toggle_focus(),
            MenuAction::ToggleTypewriter => self.toggle_typewriter(),
            MenuAction::ToggleSourceMode => self.toggle_source_mode(),
            MenuAction::ToggleRemoteImages => self.toggle_remote_images(),
            MenuAction::ZoomIn => self.zoom_step(1),
            MenuAction::ZoomOut => self.zoom_step(-1),
            MenuAction::ZoomReset => self.zoom_reset(),
            MenuAction::ToggleFullscreen => self.toggle_fullscreen(),
            MenuAction::ToggleOutline => self.toggle_outline(),
            MenuAction::Find => self.toggle_search(),
            MenuAction::Replace => self.toggle_replace(),
            MenuAction::InsertImage => self.insert_image_via_dialog(),
            MenuAction::ToggleBold => self.perform_action(Action::Doc(Intent::ToggleWrap("**"))),
            MenuAction::ToggleItalic => self.perform_action(Action::Doc(Intent::ToggleWrap("*"))),
            MenuAction::InsertLink => self.perform_action(Action::Doc(Intent::InsertLink)),
            MenuAction::OpenRecent(i) => self.open_recent(i),
            MenuAction::QuickOpen => self.toggle_quick(),
            MenuAction::Separator => {}
            MenuAction::Quit => self.confirm_close(),
            MenuAction::Settings => self.open_settings(),
        }
        self.request_redraw();
        false
    }

    pub(crate) fn copy_selection(&mut self) {
        if let Some(t) = self.editor.copy() {
            self.ports.clipboard.set(&t);
        }
    }

    pub(crate) fn cut_selection(&mut self) {
        if let Some((t, dirty)) = self.editor.cut() {
            self.ports.clipboard.set(&t);
            self.window_state.goal_x = None;
            self.after_edit(dirty);
        }
    }

    // ── 右键上下文菜单（2f）：选区/链接/块三类上下文 ─────────────────────────

    /// 当前右键菜单布局（关闭时仍有意义但调用方只在 open 时使用）：钳位进内容视口。
    fn ctx_layout(&self) -> ctx_menu::Layout {
        let vp = self.viewport();
        ctx_menu::layout(
            self.ui_state.ctx.items(),
            self.ui_state.ctx.x,
            self.ui_state.ctx.y,
            vp.x + vp.w,
            vp.y + vp.h,
            self.input_state.scale,
        )
    }

    /// 右键按下：落点在选区内保留选区，否则移动光标；链接上给出「复制链接地址」。
    /// 查找/快速打开浮层期间与 chrome 区域不弹；已打开时按新位置重开。
    pub(crate) fn on_right_click(&mut self) {
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        let vp = self.viewport();
        if self.ui_state.modal.is_some() {
            return; // 模态优先：不叠加右键菜单
        }
        if self.ui_state.quick.open || self.ui_state.search.open || self.ui_state.history.open {
            return; // 模态浮层优先：不叠加右键菜单
        }
        if my < vp.y || my >= vp.y + vp.h {
            return; // 菜单栏/状态栏区域无右键菜单
        }
        self.ui_state.chrome.close();
        let Some(off) = self.hit_offset() else {
            return;
        };
        let in_sel = self
            .editor
            .selection()
            .is_some_and(|s| s.start <= off && off < s.end);
        let (kind, link, mermaid_body, insert_start) = if in_sel {
            (CtxKind::Selection, None, None, None)
        } else {
            self.editor.set_caret(off);
            self.window_state.goal_x = None;
            // 优先级：图片（单元格内也命中）→ 表格内 link → table_ctx →
            // mermaid 围栏块（带专用菜单）→ 块级 link → 普通块
            match self.image_at(off) {
                Some((path, missing)) => {
                    if missing {
                        // 缺失图片：菜单打开时给出可见提示（DEF-T67-E1-005）。
                        self.ui_state.chrome.notice =
                            crate::i18n::t(crate::i18n::Key::NoticeImageMissing)
                                .replace("{}", &path);
                    }
                    (CtxKind::Image { path, missing }, None, None, None)
                }
                None => match self.link_at(off) {
                    Some(url) => (CtxKind::Link, Some(url), None, None),
                    None => match self.table_ctx_at(off) {
                        Some(tc) => (CtxKind::Table(tc), None, None, None),
                        None => match self.mermaid_block_info_at(off) {
                            Some((body, body_start)) if mermaid_body_is_empty(&body) => {
                                // 空块右键也把光标落到 body 起点：模板项只在这里有意义，
                                // 落在点击位置会破坏围栏结构。
                                self.editor.set_caret(body_start);
                                (
                                    CtxKind::MermaidEmptyBlock,
                                    None,
                                    Some(body),
                                    Some(body_start),
                                )
                            }
                            Some((body, _)) => (CtxKind::MermaidBlock, None, Some(body), None),
                            None => {
                                // Details 块优先于普通块：右键菜单给折叠/展开项。
                                // B3：命中 details 子块时同样归到所属头块，菜单行为一致。
                                let details_kind = self
                                    .index
                                    .blocks
                                    .iter()
                                    .find(|g| g.range.start <= off && off <= g.range.end)
                                    .and_then(|g| {
                                        let id = if matches!(&g.kind, BlockKind::Details { .. }) {
                                            g.id
                                        } else {
                                            g.details_parent?
                                        };
                                        Some(CtxKind::Details {
                                            id,
                                            expanded: self
                                                .ui_state
                                                .expanded_details
                                                .contains(&id),
                                        })
                                    });
                                (details_kind.unwrap_or(CtxKind::Block), None, None, None)
                            }
                        },
                    },
                },
            }
        };
        self.ui_state.ctx.open_at(mx, my, kind, link, mermaid_body);
        self.ui_state.ctx.mermaid_insert_start = insert_start;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 偏移所在表格的行列上下文：命中单元格直接取，竖线/分隔行取同一行最近单元格。
    /// 表头（及分隔行）`body_row` 为 None，只给列操作。
    fn table_ctx_at(&self, off: usize) -> Option<TableCtx> {
        let g =
            self.index.blocks.iter().find(|g| {
                g.kind == BlockKind::Table && g.range.start <= off && off <= g.range.end
            })?;
        let tbl = self.editor.session().table(g.id)?;
        let ncols = tbl.aligns.len().max(1);
        if let Some(flat) = active_cell_index(&tbl, off) {
            if flat < tbl.header.len() {
                return Some(TableCtx {
                    body_row: None,
                    col: flat,
                    ncols,
                });
            }
            let mut idx = flat - tbl.header.len();
            for (r, row) in tbl.rows.iter().enumerate() {
                if idx < row.len() {
                    return Some(TableCtx {
                        body_row: Some(r),
                        col: idx,
                        ncols,
                    });
                }
                idx -= row.len();
            }
            return None;
        }
        // 结构字符（竖线/分隔行）：按同一行最近单元格兜底
        let src = self.editor.session().text_slice(g.range)?;
        let rel = off.saturating_sub(g.range.start).min(src.len());
        let last_line = src.trim_end_matches(['\n', '\r']).matches('\n').count();
        let line_index = src[..rel].matches('\n').count().min(last_line);
        let body_row = if line_index >= 2 && line_index - 2 < tbl.rows.len() {
            Some(line_index - 2)
        } else {
            None
        };
        let mut best = (usize::MAX, 0usize);
        for c in 0..ncols {
            let d = table_cell_start(&src, line_index, c).abs_diff(rel);
            if d < best.0 {
                best = (d, c);
            }
        }
        Some(TableCtx {
            body_row,
            col: best.1,
            ncols,
        })
    }

    /// 偏移是否命中 mermaid 围栏代码块：命中返回 (剥掉围栏的 body 源码, body 绝对起点)。
    /// body 起点 = 开围栏行换行之后；无换行（未闭合且无 body）时返回 None。
    fn mermaid_block_info_at(&self, off: usize) -> Option<(String, usize)> {
        let g = self.index.blocks.iter().find(|g| {
            g.range.start <= off
                && off <= g.range.end
                && matches!(
                    &g.kind,
                    BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
                )
        })?;
        let (_, body) = self.editor.session().fenced_code_source(g.id)?;
        let src = self.editor.session().text_slice(g.range)?;
        let first_nl = src.find('\n')?;
        Some((body, g.range.start + first_nl + 1))
    }

    /// 左键命中空 mermaid 块：光标落到该块 body 起点，并在鼠标处打开仅含
    /// 三条模板项的菜单。返回 true 表示本次点击已被消费。
    fn try_open_mermaid_empty_menu(&mut self) -> bool {
        // 源码模式展示原始围栏、不出现卡片头与提示行：点击必须保持普通落光标，
        // 否则用户改源码时会被菜单劫持（与 Wave 5「源码模式无叠加」一致）。
        if self.source_mode() {
            return false;
        }
        let Some(off) = self.hit_offset() else {
            return false;
        };
        // 块间距点击也会被 hit_offset 归入上一块末尾；空块左键会弹菜单，
        // 因此要求点击 y 落在该块视觉高度内，避免块后空白误触。
        let vp = self.viewport();
        let y = self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y;
        let inside = self
            .index
            .blocks
            .iter()
            .find(|g| {
                g.range.start <= off
                    && off <= g.range.end
                    && matches!(
                        &g.kind,
                        BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
                    )
            })
            .is_some_and(|g| y >= g.top && y < g.top + self.block_height(g));
        if !inside {
            return false;
        }
        let Some((body, body_start)) = self.mermaid_block_info_at(off) else {
            return false;
        };
        if !mermaid_body_is_empty(&body) {
            return false;
        }
        // 与普通鼠标单击落光标一致：先断组，模板插入才会成为独立撤销步。
        self.editor.break_undo_group();
        if !self.editor.set_caret(body_start) {
            return false;
        }
        self.window_state.goal_x = None;
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        self.ui_state
            .ctx
            .open_at(mx, my, CtxKind::MermaidEmptyBlock, None, Some(body));
        self.ui_state.ctx.mermaid_insert_start = Some(body_start);
        self.set_ime_area();
        self.request_redraw();
        true
    }

    /// 行内 `<details>` 的 summary 点击（B3）：命中 summary 可见文本 → 切换折叠。
    /// 纯视图态：不移动光标、不写文档字节、不产生撤销步。
    fn try_toggle_inline_details_on_click(&mut self) -> bool {
        let Some(off) = self.hit_offset() else {
            return false;
        };
        let Some((id, range, is_block_details)) = self
            .index
            .blocks
            .iter()
            .find(|g| !self.block_hidden(g) && g.range.start <= off && off < g.range.end)
            .map(|g| (g.id, g.range, matches!(g.kind, BlockKind::Details { .. })))
        else {
            return false;
        };
        // 块级 details 头块另有折叠入口；这里只处理块内行内 details。
        // （头块自身的 `<details>` 未闭合，inline 解析可能产出 Details 节点，
        //  若不排除会误把点击记到「行内折叠」集合里，块级点击就失效了。）
        if is_block_details {
            return false;
        }
        let text = self.editor.session().text_slice(range).unwrap_or_default();
        if !text.contains("<details") {
            return false;
        }
        let Some(nodes) = self.editor.session().block_inlines(id) else {
            return false;
        };
        let spans = inline_details_spans(&nodes, range);
        for s in spans {
            let Some((ss, se)) = s.summary else { continue };
            let (abs_s, abs_e) = (range.start + ss, range.start + se);
            if abs_s <= off && off < abs_e {
                let key = (id, s.key);
                if !self.ui_state.expanded_inline_details.remove(&key) {
                    self.ui_state.expanded_inline_details.insert(key);
                } else {
                    // 收起：光标若在正文内（隐藏区）则移到 summary 文本起点。
                    let caret = self.editor.caret();
                    let body = (range.start + s.body.0, range.start + s.body.1);
                    if body.0 <= caret && caret < body.1 {
                        self.editor.set_caret(abs_s);
                    }
                }
                // 折叠态进入排版键：重算几何 + 重绘（缓存按基础键自然失配）。
                self.recompute_geometry();
                self.request_redraw();
                return true;
            }
        }
        false
    }

    /// B3 兜底：光标落在 details 隐藏区（任意路径：撤销/搜索/跳转）时展开归属。
    /// 行内 details 的正文若包含光标，同样展开该行内 details。
    pub(crate) fn sync_details_at_caret(&mut self) {
        let caret = self.effective_caret();
        // 光标停在 details 闭标签行（纯 chrome，不属于任何块）：非选区状态下
        // 吸附到前一个可见块末尾，避免「光标悬空 / 画到文档尾」。
        if self.editor.selection().is_none()
            && !self
                .index
                .blocks
                .iter()
                .any(|g| g.range.start <= caret && caret <= g.range.end)
        {
            let snap = {
                let s = self.editor.session();
                let line = s.line_of_offset(caret);
                let ls = s.offset_of_line(line);
                let le = s.offset_of_line(line + 1);
                let is_close = s
                    .text_slice(veil_core::Range::new(ls, le))
                    .is_some_and(|t| t.to_ascii_lowercase().contains("</details"));
                if is_close {
                    self.index
                        .blocks
                        .iter()
                        .rev()
                        .find(|g| g.range.end <= ls && !self.block_hidden(g))
                        .and_then(|g| {
                            s.text_slice(g.range)
                                .map(|t| g.range.start + t.trim_end_matches(['\n', '\r']).len())
                        })
                } else {
                    None
                }
            };
            if let Some(target) = snap {
                self.editor.set_caret(target);
                return;
            }
        }
        let (id, range, hidden) = {
            let Some(g) = self
                .index
                .blocks
                .iter()
                .find(|g| g.range.start <= caret && caret < g.range.end)
                .or_else(|| {
                    self.index
                        .blocks
                        .iter()
                        .rev()
                        .find(|g| g.range.start <= caret)
                })
            else {
                return;
            };
            (g.id, g.range, self.block_hidden(g))
        };
        if hidden {
            let mut p = self
                .index
                .blocks
                .iter()
                .find(|b| b.id == id)
                .and_then(|b| b.details_parent);
            let mut changed = false;
            while let Some(pid) = p {
                changed |= self.ui_state.expanded_details.insert(pid);
                p = self
                    .index
                    .blocks
                    .iter()
                    .find(|b| b.id == pid)
                    .and_then(|h| h.details_parent);
            }
            if changed {
                self.recompute_geometry();
            }
            return;
        }
        let text = self.editor.session().text_slice(range).unwrap_or_default();
        if !text.contains("<details") {
            return;
        }
        let Some(nodes) = self.editor.session().block_inlines(id) else {
            return;
        };
        let mut changed = false;
        for s in inline_details_spans(&nodes, range) {
            let body = (range.start + s.body.0, range.start + s.body.1);
            if body.0 <= caret && caret < body.1 {
                changed |= self.ui_state.expanded_inline_details.insert((id, s.key));
            }
        }
        if changed {
            self.recompute_geometry();
        }
    }

    /// Details 块折叠/展开：命中 Details 块 → 根据展开态和 y 位置决定是否切换。
    /// 收起态：整块可视区（仅 summary 一行）点击都触发展开；
    /// 展开态：只 summary 行（y ∈ [top, top + line_h)）触发收起，正文区落光标。
    /// 返回 true 表示本次点击已被折叠逻辑消费（不走后续 point_caret）。
    fn try_toggle_details_on_click(&mut self) -> bool {
        if self.try_toggle_inline_details_on_click() {
            return true;
        }
        let Some(off) = self.hit_offset() else {
            return false;
        };
        let Some(g) = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= off && off <= g.range.end)
        else {
            return false;
        };
        let BlockKind::Details { .. } = &g.kind else {
            return false;
        };
        let is_expanded = self.ui_state.expanded_details.contains(&g.id);
        let vp = self.viewport();
        let y = self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y;
        let block_h = self.block_height(g);
        if !(y >= g.top && y < g.top + block_h) {
            return false; // 点击在块后间距或更下方 → 不折叠
        }
        if !is_expanded {
            // 收起态：点击整块（只有一行可视）= 展开
            self.toggle_details(g.id);
            return true;
        }
        // 展开态：点击在 summary 行 = 收起，否则落光标。
        // y / g.top 是物理像素，`doc_line_height` 是逻辑基准，必须乘 scale；
        // 否则非 100% DPI 下 summary 行会被判成正文区，点击无法收起。
        let line_h = self.engine.doc_line_height() * self.input_state.scale;
        if y < g.top + line_h {
            self.toggle_details(g.id);
            return true;
        }
        false
    }

    /// 命中偏移所在块的 inline 节点（表格按单元格取，其余取块 inlines）。
    fn inlines_at_offset(&self, offset: usize) -> Option<Vec<InlineNode>> {
        let g = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= offset && offset < g.range.end)?;
        match g.kind {
            BlockKind::Table => {
                let tbl = self.editor.session().table(g.id)?;
                let flat = active_cell_index(&tbl, offset)?;
                let cell = if flat < tbl.header.len() {
                    tbl.header.get(flat)?
                } else {
                    let mut idx = flat - tbl.header.len();
                    let mut found: Option<_> = None;
                    for row in &tbl.rows {
                        if idx < row.len() {
                            found = row.get(idx);
                            break;
                        }
                        idx -= row.len();
                    }
                    found?
                };
                Some(cell.inlines.clone())
            }
            _ => self.editor.session().block_inlines(g.id),
        }
    }

    /// 命中偏移处的 inline 链接 URL（含标记区命中；引用式链接无 url 区间不识别）。
    /// 扩展：表格单元格内的 inline 链接也能命中（`block_inlines(Table)` 恒为空，
    /// 需从 `table.cell.inlines` 逐级检索）。
    fn link_at(&self, offset: usize) -> Option<String> {
        let nodes = self.inlines_at_offset(offset)?;
        fn find(nodes: &[InlineNode], off: usize) -> Option<&InlineNode> {
            nodes.iter().find_map(|n| {
                if n.range.start <= off && off < n.range.end {
                    if n.kind == InlineKind::Link && n.url.is_some() {
                        return Some(n);
                    }
                    find(&n.children, off)
                } else {
                    None
                }
            })
        }
        let url = find(&nodes, offset)?.url?;
        self.editor.session().text_slice(url)
    }

    /// 命中偏移处的图片源与缺失状态（DEF-T67-E1-005）。
    /// 支持 Markdown `![alt](src)` 与白名单自闭合 `<img src="…">`；
    /// 远程/内联图片视为“存在”，本地图片按文档目录解析后探测文件。
    pub(crate) fn image_at(&self, offset: usize) -> Option<(String, bool)> {
        let nodes = self.inlines_at_offset(offset)?;
        fn find(nodes: &[InlineNode], off: usize) -> Option<&InlineNode> {
            nodes.iter().find_map(|n| {
                if n.range.start <= off && off < n.range.end {
                    if matches!(n.kind, InlineKind::Image)
                        || matches!(
                            &n.kind,
                            InlineKind::HtmlSelfClosed {
                                tag: HtmlTag::Img,
                                ..
                            }
                        )
                    {
                        return Some(n);
                    }
                    find(&n.children, off)
                } else {
                    None
                }
            })
        }
        let node = find(&nodes, offset)?;
        let src = match &node.kind {
            InlineKind::Image => {
                let range = node.url?;
                Some(self.editor.session().text_slice(range)?)
            }
            InlineKind::HtmlSelfClosed {
                tag: HtmlTag::Img,
                attrs,
            } => Some(attrs.src.clone().unwrap_or_default()),
            _ => return None,
        };
        let src = src?.trim().to_string();
        if src.is_empty() {
            return None;
        }
        let missing = match crate::paint::local_image_path(&src, self.doc.path.as_deref()) {
            Some(path) => !path.exists(),
            None => false,
        };
        Some((src, missing))
    }

    /// 命中偏移处的 `<abbr title="…">` 文本（悬停提示内容）。表格单元格同样支持；
    /// 空 title 不产出提示（HTML 语义上等同于无提示）。
    fn abbr_title_at(&self, offset: usize) -> Option<String> {
        let nodes = self.inlines_at_offset(offset)?;
        fn find(nodes: &[InlineNode], off: usize) -> Option<&str> {
            nodes.iter().find_map(|n| {
                if n.range.start <= off && off < n.range.end {
                    if let InlineKind::Html {
                        tag: HtmlTag::Abbr,
                        attrs,
                    } = &n.kind
                    {
                        if let Some(t) = attrs.title.as_deref().filter(|t| !t.is_empty()) {
                            return Some(t);
                        }
                    }
                    find(&n.children, off)
                } else {
                    None
                }
            })
        }
        find(&nodes, offset).map(str::to_owned)
    }

    /// 命中偏移处的脚注引用 id（`[^id]` 的整段 `range` 包含判定）。
    /// 渲染态 `[^id]` 是替换段 `[n]`，点击 `[n]` 逆映射出的 doc 偏移仍落在节点
    /// `range` 内，因此这里只能按源码区间判命中，不能拿可见文本 `[n]` 反查。
    fn footnote_ref_id_at(&self, offset: usize) -> Option<String> {
        let g = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= offset && offset < g.range.end)?;
        let nodes = self.editor.session().block_inlines(g.id)?;
        fn find(nodes: &[InlineNode], off: usize) -> Option<Range> {
            nodes.iter().find_map(|n| {
                if n.range.start <= off && off < n.range.end {
                    if n.kind == InlineKind::FootnoteRef {
                        return Some(n.content);
                    }
                    find(&n.children, off)
                } else {
                    None
                }
            })
        }
        let content = find(&nodes, offset)?;
        self.editor.session().text_slice(content)
    }

    /// 文档顺序中首个 `[^id]` 引用节点起点（多引用时规范为「回到第一次引用」）。
    fn first_footnote_ref(&self, id: &str) -> Option<usize> {
        // 块按索引顺序、inline 按节点顺序递归，即文档顺序。
        let session = self.editor.session();
        for g in &self.index.blocks {
            let Some(nodes) = session.block_inlines(g.id) else {
                continue;
            };
            for n in &nodes {
                if let Some(p) = self.find_inline_ref(n, id) {
                    return Some(p);
                }
            }
        }
        None
    }

    fn find_inline_ref(&self, n: &InlineNode, id: &str) -> Option<usize> {
        if n.kind == InlineKind::FootnoteRef
            && self.editor.session().text_slice(n.content).as_deref() == Some(id)
        {
            return Some(n.range.start);
        }
        n.children.iter().find_map(|c| self.find_inline_ref(c, id))
    }

    /// 应用内跳转统一落点：设置光标 + 滚到可见。只动光标标记与滚动量，
    /// 不编辑文档、不开撤销步（Ctrl+单击跳转不是编辑行为）。
    fn jump_to_offset(&mut self, target: usize) {
        if !self.editor.set_caret(target) {
            return;
        }
        self.window_state.goal_x = None;
        self.scroll_to_caret();
        self.set_ime_area();
        self.request_redraw();
    }

    /// Ctrl+单击脚注引用 → 定义内容起点。命中即消费点击（返回 true），
    /// 未定义时提示但光标保持原位——用户预期是「跳过去」，不是「落光标」。
    fn follow_footnote_ref(&mut self, offset: usize) -> bool {
        let Some(id) = self.footnote_ref_id_at(offset) else {
            return false;
        };
        let def = self.editor.session().footnote_definition(&id);
        match def {
            Some(def) => self.jump_to_offset(def.start),
            None => {
                self.ui_state.chrome.notice = t(I18nKey::NoticeFootnoteDefMissing).to_string();
                self.request_redraw();
            }
        }
        true
    }

    /// Ctrl+单击定义块末尾 `↩` → 该 id 的首个引用（`↩` 为零宽插入段，锚在块末）。
    fn follow_footnote_back(&mut self, offset: usize) -> bool {
        // 文末无换行时块末命中可能等于 range.end，故对最后一个块补一次回退查找。
        let Some(idx) = self
            .index
            .blocks
            .iter()
            .position(|g| g.range.start <= offset && offset < g.range.end)
            .or_else(|| {
                self.index
                    .blocks
                    .iter()
                    .rposition(|g| g.range.end == offset)
            })
        else {
            return false;
        };
        let (kind, range) = {
            let g = &self.index.blocks[idx];
            (g.kind.clone(), g.range)
        };
        if kind != BlockKind::FootnoteDef {
            return false;
        }
        let text = self.editor.session().text_slice(range).unwrap_or_default();
        // 排版 body 会剥掉末尾单个 `\n`（`↩` 插在 body.len() 处），故块末正文
        // 末尾 = range.end - 1；命中逆映射只可能得到该点或 range.end（文末无换行）。
        // 判 `offset >= body_end` 即命中返回标记，不必再猜可见字符宽度。
        let body_end = range.start + text.strip_suffix('\n').unwrap_or(&text).len();
        if offset < body_end {
            return false;
        }
        let Some(id) = footnote_def_id(&text) else {
            return false;
        };
        let target = self.first_footnote_ref(id);
        match target {
            Some(start) => self.jump_to_offset(start),
            None => {
                self.ui_state.chrome.notice = t(I18nKey::NoticeFootnoteRefMissing).to_string();
                self.request_redraw();
            }
        }
        true
    }

    /// Ctrl+单击 `[文本](#slug)` → 同文档标题起点。
    /// slug 先按导出口径（`heading_slugs` 去重结果）原样匹配，未命中再按
    /// `heading_slug` 归一化匹配，容忍用户手写 `#Hello World` 这类非导出值。
    fn jump_to_anchor(&mut self, url: &str) {
        let slug = url.trim().trim_start_matches('#');
        let target = {
            let session = self.editor.session();
            let mut starts = Vec::new();
            let mut texts = Vec::new();
            for g in &self.index.blocks {
                if !matches!(
                    g.kind,
                    BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
                ) {
                    continue;
                }
                if let Some((_, text)) = session.heading(g.id) {
                    starts.push(g.range.start);
                    texts.push(text);
                }
            }
            let slugs = heading_slugs(texts.iter().map(String::as_str));
            let exact = slugs.iter().position(|s| s == slug);
            exact
                .or_else(|| {
                    let norm = heading_slug(slug);
                    slugs.iter().position(|s| *s == norm)
                })
                .map(|i| starts[i])
        };
        match target {
            Some(start) => self.jump_to_offset(start),
            None => {
                self.ui_state.chrome.notice = t(I18nKey::NoticeAnchorMissing).to_string();
                self.request_redraw();
            }
        }
    }

    /// 用系统默认程序打开链接：Ctrl+左键与右键菜单「在浏览器中打开」共用此路径。
    /// 协议白名单在此统一校验（不信任文档内容）；`#anchor` 属应用内跳转（另一任务），
    /// 静默忽略，绝不交给系统 shell。失败原因直接写状态栏，不打断编辑。
    pub(crate) fn open_link(&mut self, url: &str) {
        let trimmed = url.trim();
        if trimmed.starts_with('#') {
            return; // 应用内锚点：由后续任务处理，这里不弹提示也不调端口
        }
        // 裸 `www.` 链接在源码里没有协议，打开前补齐（否则被协议白名单拒绝）
        let target = crate::normalize_link_url(trimmed);
        if !crate::url_is_openable(&target) {
            self.ui_state.chrome.notice = tf(I18nKey::NoticeLinkUnsupported, &[&target]);
            self.request_redraw();
            return;
        }
        if let Err(e) = self.ports.url_opener.open_url(&target) {
            self.ui_state.chrome.notice = tf(I18nKey::NoticeLinkFailed, &[&e.to_string()]);
        }
        self.request_redraw();
    }

    /// 执行右键菜单动作（菜单随后由调用方关闭；CopyLink 载荷读自 ctx.link）。
    fn ctx_exec(&mut self, action: CtxAction) {
        match action {
            CtxAction::Separator => {}
            CtxAction::Cut => self.cut_selection(),
            CtxAction::Copy => self.copy_selection(),
            CtxAction::Paste => self.paste_clipboard(),
            CtxAction::SelectAll => self.perform_action(Action::Doc(Intent::SelectAll)),
            CtxAction::CopyLink => {
                if let Some(u) = self.ui_state.ctx.link.clone() {
                    // 复制出去的是可直接使用的目标：`www.` 补全协议；
                    // 非白名单 scheme 与打开同口径拒绝，不把危险地址带出程序。
                    let target = crate::normalize_link_url(&u);
                    if veil_core::link_url_is_safe(&target) {
                        self.ports.clipboard.set(&target);
                    } else {
                        self.ui_state.chrome.notice =
                            tf(I18nKey::NoticeLinkUnsupported, &[&target]);
                        self.request_redraw();
                    }
                }
            }
            CtxAction::OpenLink => {
                if let Some(u) = self.ui_state.ctx.link.clone() {
                    self.open_link(&u);
                }
            }
            CtxAction::InsertImage => self.insert_image_via_dialog(),
            CtxAction::TableInsertRowAbove
            | CtxAction::TableInsertRowBelow
            | CtxAction::TableDeleteRow
            | CtxAction::TableInsertColumnLeft
            | CtxAction::TableInsertColumnRight
            | CtxAction::TableDeleteColumn
            | CtxAction::TableAlignLeft
            | CtxAction::TableAlignCenter
            | CtxAction::TableAlignRight => self.apply_table_action(action),
            // Mermaid v0：预览 / 复制源码 / 复制链接（读 ctx.mermaid_body）
            CtxAction::MermaidPreview
            | CtxAction::MermaidCopySource
            | CtxAction::MermaidCopyLink => {
                let Some(body) = self.ui_state.ctx.mermaid_body.clone() else {
                    return;
                };
                match action {
                    CtxAction::MermaidPreview => {
                        let url = veil_core::mermaid_live_url(&body);
                        let _ = self.ports.url_opener.open_url(&url);
                    }
                    CtxAction::MermaidCopySource => {
                        self.ports.clipboard.set(&body);
                    }
                    CtxAction::MermaidCopyLink => {
                        let url = veil_core::mermaid_live_url(&body);
                        self.ports.clipboard.set(&url);
                    }
                    _ => unreachable!(),
                }
            }
            // Mermaid v0：插入预设模板到当前光标位置（空块时为 body 起点）
            CtxAction::MermaidInsertFlowchart => {
                self.insert_mermaid_template(veil_core::FLOWCHART_TEMPLATE);
            }
            CtxAction::MermaidInsertSequence => {
                self.insert_mermaid_template(veil_core::SEQUENCE_TEMPLATE);
            }
            CtxAction::MermaidInsertClass => {
                self.insert_mermaid_template(veil_core::CLASS_TEMPLATE);
            }
            // Details 块折叠 / 展开：从 ctx.details_id 拿目标块 id（打开时由 CtxKind 注入）。
            CtxAction::FoldDetails => {
                if let Some(id) = self.ui_state.ctx.details_id {
                    if self.ui_state.expanded_details.contains(&id) {
                        self.toggle_details(id);
                    }
                }
            }
            CtxAction::UnfoldDetails => {
                if let Some(id) = self.ui_state.ctx.details_id {
                    if !self.ui_state.expanded_details.contains(&id) {
                        self.toggle_details(id);
                    }
                }
            }
            // 图片专用动作（DEF-T67-E1-005）：复制源路径；重新加载清除失败缓存后重绘。
            CtxAction::CopyImagePath => {
                if let Some(path) = self.ui_state.ctx.image_path.clone() {
                    self.ports.clipboard.set(&path);
                    self.ui_state.chrome.notice =
                        crate::i18n::t(crate::i18n::Key::CtxCopyImagePath).to_string();
                }
            }
            CtxAction::ReloadImage => {
                let Some(src) = self.ui_state.ctx.image_path.clone() else {
                    return;
                };
                let key = match crate::paint::local_image_path(&src, self.doc.path.as_deref()) {
                    Some(path) => path.to_string_lossy().into_owned(),
                    None => src,
                };
                self.cache.invalidate_image(&key);
                self.request_redraw();
            }
            // 提示项无动作：文案已在菜单中可见。
            CtxAction::ImageMissingHint => {}
        }
        self.request_redraw();
    }

    /// 插入 mermaid 模板：非空块沿用「在光标处插入」；空块插入 body 起点，
    /// 若插入点后紧跟闭合围栏/注释（非换行）则补一个换行，避免模板末行与
    /// 围栏粘连。光标最终停在模板正文末尾，整次插入为单个撤销步。
    fn insert_mermaid_template(&mut self, template: &str) {
        let Some(start) = self.ui_state.ctx.mermaid_insert_start else {
            self.perform_action(Action::Doc(Intent::InsertText(template.into())));
            return;
        };
        let len = self.editor.session().text_len();
        let next = if start < len {
            self.editor
                .session()
                .text_slice(Range::new(start, start + 1))
        } else {
            None
        };
        let needs_nl = start < len && !matches!(next.as_deref(), Some("\n") | Some("\r"));
        let mut text = String::from(template);
        if needs_nl {
            text.push('\n');
        }
        self.perform_action(Action::Doc(Intent::InsertText(text)));
        self.editor.set_caret(start + template.len());
    }

    /// 表格结构操作（B2）：按打开菜单时的行列上下文生成新源码，经唯一门面
    /// `replace_table_block` 整块替换（一次 undo 步），并落到目标单元格内容起点。
    fn apply_table_action(&mut self, action: CtxAction) {
        let Some(tc) = self.ui_state.ctx.table else {
            return;
        };
        let caret = self.editor.caret();
        let Some(block) = self
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table && g.range.start <= caret && caret <= g.range.end)
            .map(|g| g.range)
        else {
            return;
        };
        let src = self.editor.session().text_slice(block).unwrap_or_default();
        // 对齐只改分隔行该列标记：等长替换，光标停留在原偏移（相对块起点不变）。
        let align = match action {
            CtxAction::TableAlignLeft => Some(CellAlign::Left),
            CtxAction::TableAlignCenter => Some(CellAlign::Center),
            CtxAction::TableAlignRight => Some(CellAlign::Right),
            _ => None,
        };
        if let Some(align) = align {
            let Some(new_src) = table_set_align(&src, tc.col, align) else {
                return;
            };
            if new_src == src {
                return;
            }
            let caret_in_block = caret.saturating_sub(block.start).min(new_src.len());
            if let Some(dirty) = self
                .editor
                .replace_table_block(block, new_src, caret_in_block)
            {
                self.after_edit(dirty);
            }
            return;
        }
        let row = tc.body_row.unwrap_or(0);
        let new_src = match action {
            CtxAction::TableInsertRowAbove => table_insert_row(&src, row, true),
            CtxAction::TableInsertRowBelow => table_insert_row(&src, row, false),
            CtxAction::TableDeleteRow => table_delete_row(&src, row),
            CtxAction::TableInsertColumnLeft => table_insert_column(&src, tc.col),
            CtxAction::TableInsertColumnRight => table_insert_column(&src, tc.col + 1),
            CtxAction::TableDeleteColumn => {
                if tc.ncols <= 1 {
                    return;
                }
                table_delete_column(&src, tc.col)
            }
            _ => return,
        };
        if new_src == src {
            return;
        }
        let last_line = new_src.trim_end_matches(['\n', '\r']).matches('\n').count();
        let (line, col) = match action {
            // 插入行 → 新行同列内容起点
            CtxAction::TableInsertRowAbove => ((2 + row).min(last_line), tc.col),
            CtxAction::TableInsertRowBelow => ((3 + row).min(last_line), tc.col),
            // 删除行 → 同列上一行；无上一行则下一个 body 行；再无可落到表头同列
            CtxAction::TableDeleteRow => {
                let line = if row > 0 {
                    2 + row - 1
                } else if last_line >= 2 {
                    2
                } else {
                    0
                };
                (line.min(last_line), tc.col)
            }
            // 列操作 → 当前行（表头命中则表头）的新列内容起点
            CtxAction::TableInsertColumnLeft => (tc.body_row.map_or(0, |r| 2 + r), tc.col),
            CtxAction::TableInsertColumnRight => (tc.body_row.map_or(0, |r| 2 + r), tc.col + 1),
            // 删除列 → 当前行删除后的同序（末列时取新的末列）
            CtxAction::TableDeleteColumn => {
                (tc.body_row.map_or(0, |r| 2 + r), tc.col.min(tc.ncols - 2))
            }
            _ => return,
        };
        let caret_in_block = table_cell_start(&new_src, line, col);
        if let Some(dirty) = self
            .editor
            .replace_table_block(block, new_src, caret_in_block)
        {
            self.after_edit(dirty);
        }
    }

    /// 表格列宽拖动（B2）：在表格内部列分隔线 ±4px 内按下 → 进入拖动会话。
    /// 纯视图态：不修改文档字节、不进入撤销栈；覆盖按块 id 保存。
    pub(crate) fn try_start_col_drag(&mut self) -> bool {
        let vp = self.viewport();
        let scale = self.input_state.scale;
        let x = self.input_state.mouse.x as f32 - self.content_x();
        let y = self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y;
        if x < 0.0 || y < 0.0 {
            return false;
        }
        let (id, kind, top) = {
            let Some(g) = self.index.blocks.iter().rev().find(|g| g.top <= y) else {
                return false;
            };
            (g.id, g.kind.clone(), g.top)
        };
        if !matches!(kind, BlockKind::Table) {
            return false;
        }
        self.ensure_block(id);
        let Some(t) = self.cache.tables.get(&id) else {
            return false;
        };
        // 容器内表格：列边界相对网格原点，命中前扣除缩进。
        let lx = x - t.x;
        let ly = y - top;
        if ly > t.height || t.cols.len() < 2 {
            return false;
        }
        let tol = 4.0 * scale;
        let mut acc = 0.0f32;
        let mut hit = None;
        for (i, cw) in t.cols.iter().enumerate() {
            acc += *cw;
            // 仅内部边界可拖（最右边界是表宽，等价于整体缩放，不在本项范围）
            if i + 1 < t.cols.len() && (lx - acc).abs() <= tol {
                hit = Some((i, *cw));
                break;
            }
        }
        let Some((col, base_w)) = hit else {
            return false;
        };
        let cols = t.cols.clone();
        self.ui_state.tables.widths.entry(id).or_insert(cols);
        self.ui_state.tables.drag = Some((id, col, lx, base_w));
        true
    }

    /// 拖动中：按鼠标位移更新目标列宽（下限 40 逻辑 px），并请求重绘。
    pub(crate) fn drag_col_to(&mut self) {
        let Some((id, col, x0, base_w)) = self.ui_state.tables.drag else {
            return;
        };
        let x = self.input_state.mouse.x as f32 - self.content_x();
        let min = crate::view::table::MIN_COL_WIDTH * self.input_state.scale;
        let indent = self.cache.tables.get(&id).map_or(0.0, |m| m.value.x);
        let x = x - indent;
        let w = (base_w + (x - x0)).max(min);
        if let Some(ws) = self.ui_state.tables.widths.get_mut(&id) {
            if let Some(slot) = ws.get_mut(col) {
                *slot = w;
            }
        }
        self.request_redraw();
    }

    /// 松开左键：结束列宽拖动会话（覆盖保留）。
    pub(crate) fn end_col_drag(&mut self) -> bool {
        self.ui_state.tables.drag.take().is_some()
    }

    /// 左键按下路由（模态 > 右键菜单 > 菜单 > 菜单栏 > 滚动条 > 状态栏 > 正文光标）。
    /// 返回值恒为 false：退出统一由模态结论写 `exit_requested`。
    /// 菜单打开时点击外部只关菜单、不移动光标。
    pub(crate) fn on_left_click(&mut self) -> bool {
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        // 设置模态优先（优先级高于普通 modal）：只判定按钮/字段命中，遮罩/空白点击一律吞掉
        if self.ui_state.settings.is_some() {
            let l = self.settings_layout();
            let hit = self
                .ui_state
                .settings
                .as_mut()
                .and_then(|s| s.state.on_click((mx, my), &l, &mut s.form));
            if let Some(id) = hit {
                self.resolve_settings(id);
            }
            self.request_redraw();
            return false;
        }
        // 模态优先：只判定按钮命中，遮罩/空白点击一律吞掉且不关闭（须显式作答）
        if self.ui_state.modal.is_some() {
            let l = self.modal_layout();
            let hit = self
                .ui_state
                .modal
                .as_mut()
                .and_then(|a| a.state.on_click((mx, my), &l));
            if let Some(id) = hit {
                self.resolve_modal(id);
            }
            self.request_redraw();
            return false;
        }
        let scale = self.input_state.scale;
        let vp = self.viewport();
        let win_w = vp.w;
        // 版本历史（B8a）：左侧列表 + 正文区只读预览；面板内外点击都不落正文光标
        if self.ui_state.history.open {
            let l = self.history_layout();
            let count = self.ui_state.history.entries.len();
            let top = history::first_visible(self.ui_state.history.scroll, scale);
            match history::hit(&l, top, count, mx, my, scale) {
                history::Hit::Row(i) => self.history_select(i),
                history::Hit::Action(a) => self.history_action(a),
                history::Hit::Background | history::Hit::Outside => {}
            }
            self.request_redraw();
            return false;
        }
        // 右键菜单打开中：命中项执行；其余位置（含面板内空白）关闭并吞掉
        if self.ui_state.ctx.open {
            let l = self.ctx_layout();
            let hit = ctx_menu::hit(&l, mx, my);
            if let Some(i) = hit {
                if let Some(a) = self.ui_state.ctx.action(i) {
                    self.ctx_exec(a);
                }
            }
            self.ui_state.ctx.close();
            self.request_redraw();
            return false;
        }
        if let Some(open) = self.ui_state.chrome.open {
            let recent = self.recent_labels();
            if let Some(item) = chrome::hit_popup_item(mx, my, open, scale, &recent, win_w) {
                if let Some(action) = chrome::action_at(open, item, &recent) {
                    // 禁用项：吞掉点击、菜单保持打开（与原生菜单一致）
                    if !chrome::action_enabled(action, self.menu_flags()) {
                        return false;
                    }
                    self.ui_state.chrome.close();
                    return self.perform_menu(action);
                }
                self.ui_state.chrome.close();
                return false;
            }
            if chrome::popup_contains(mx, my, open, scale, &recent, win_w) {
                return false; // 弹窗内边距：吞掉点击
            }
            self.ui_state.chrome.close();
            if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
                if m != open {
                    self.ui_state.chrome.open = Some(m);
                }
                self.request_redraw();
                return false;
            }
            self.request_redraw();
            return false;
        }
        if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
            // 菜单与快速打开/查找栏互斥：开启菜单前先收掉轻量浮层
            self.close_quick();
            self.close_search();
            self.ui_state.chrome.toggle(m);
            self.request_redraw();
            return false;
        }
        // 快速打开浮层（阶段 1）：浮在内容与查找栏之上
        if let Some(l) = self.quick_rect() {
            let count = self.ui_state.quick.filtered.len();
            match quick_open::hit(&l, self.ui_state.quick.top, count, mx, my) {
                Some(i) => {
                    self.ui_state.quick.selected = i;
                    self.quick_accept();
                }
                None if l.panel.contains(mx, my) => {} // 面板内空白：吞掉点击
                None => self.close_quick(),            // 面板外：关闭浮层
            }
            return false;
        }
        // 查找栏（P3-3）：浮在内容之上，先于滚动条/大纲/正文命中
        if let Some(l) = self.search_rect() {
            let hit = search::hit(&l, scale, self.ui_state.search.replace_open, mx, my);
            // active 态：按下即记录，松开（app.rs 的 Released 分支）清空。
            if let Some(h) = hit {
                self.ui_state.search.pressed = Some(h);
                self.request_redraw();
            }
            match hit {
                Some(search::Hit::Close) => {
                    self.close_search();
                    return false;
                }
                Some(search::Hit::Prev) => {
                    self.search_prev();
                    return false;
                }
                Some(search::Hit::Next) => {
                    self.search_next();
                    return false;
                }
                Some(search::Hit::ToggleCase) => {
                    self.search_toggle_case();
                    return false;
                }
                Some(search::Hit::QueryText) => {
                    self.ui_state.search.focus = search::Field::Query;
                    self.set_ime_area();
                    self.request_redraw();
                    return false;
                }
                Some(search::Hit::ReplaceText) => {
                    self.ui_state.search.focus = search::Field::Replace;
                    self.set_ime_area();
                    self.request_redraw();
                    return false;
                }
                Some(search::Hit::ReplaceOne) => {
                    self.replace_current();
                    return false;
                }
                Some(search::Hit::ReplaceAll) => {
                    self.replace_all();
                    return false;
                }
                Some(search::Hit::Bar) => return false, // 栏内空白：吞掉点击
                None => {}
            }
        }
        let doc_h = self.doc_height();
        if chrome::scrollbar_hit(vp, doc_h, mx, my, scale) {
            if let Some((ty, th)) =
                chrome::scroll_thumb(vp, doc_h, self.window_state.scroll_y, scale)
            {
                if my >= ty && my < ty + th {
                    self.ui_state.chrome.scroll_drag = Some(my - ty);
                } else {
                    let dir = if my < ty { -1 } else { 1 };
                    let new = chrome::page_scroll(vp, doc_h, self.window_state.scroll_y, dir);
                    self.window_state.goal_y = new;
                    if !self.window_state.smooth {
                        self.window_state.scroll_y = self.window_state.goal_y;
                    }
                    self.set_ime_area();
                    self.request_redraw();
                }
            }
            return false;
        }
        if my < vp.y || my >= vp.y + vp.h {
            return false; // 状态栏等 chrome 区域：不落光标
        }
        // 大纲面板：点击条目跳转；面板内其它位置吞掉点击（不落光标）
        if let Some(panel) = self.outline_rect() {
            if mx >= panel.x && mx < panel.x + panel.w {
                if let Some(i) = outline::hit_row(
                    panel,
                    self.ui_state.outline.entries.len(),
                    self.ui_state.outline.scroll,
                    scale,
                    mx,
                    my,
                ) {
                    self.jump_outline(i);
                }
                return false;
            }
        }
        // Ctrl+左键：应用内跳转（`#anchor` → 脚注引用 → 定义末尾返回）优先，
        // 其余 http/https/mailto 才交给系统默认程序。必须在此提前返回——
        // 后续落光标/双击选词会移动光标、改选区，甚至开启撤销步，
        // 而「跳转/打开链接」都不应产生任何编辑副作用（文档真值保持只读）。
        if self.input_state.mods.ctrl {
            if let Some(off) = self.hit_offset() {
                let link = self.link_at(off);
                if let Some(url) = link.as_deref().filter(|u| u.trim().starts_with('#')) {
                    self.jump_to_anchor(url);
                    return false;
                }
                if self.follow_footnote_ref(off) || self.follow_footnote_back(off) {
                    return false;
                }
                if let Some(url) = link {
                    self.open_link(&url);
                    return false;
                }
            }
        }
        // 表格列宽拖动（B2）：优先于落光标；命中分隔线则吞掉本次点击。
        if self.try_start_col_drag() {
            self.request_redraw();
            return false;
        }
        // 空 Mermaid 块提示点击：空块没有其它点击语义（无正文可选、非表格、
        // 无任务框），因此允许点击整块卡片（含提示行）触发模板菜单；非空块
        // 仍走普通落光标/双击选词，分支顺序不变。
        if self.try_open_mermaid_empty_menu() {
            return false;
        }
        // Details 块折叠/展开：收起态点击整块 = 展开（唯一交互入口）；
        // 展开态点击 summary 行 = 收起（y ∈ [top, top + line_h)），
        // 正文区点击照常落光标。优先级高于 point_caret。
        if self.try_toggle_details_on_click() {
            return false;
        }
        self.window_state.dragging = true;
        if self.try_toggle_task() {
            self.window_state.dragging = false; // 方框切换：不进入拖选
            return false;
        }
        // 与键盘移动/扩选断组对齐：鼠标单击/双击/三击落正文均开启新撤销步
        self.editor.break_undo_group();
        match self.click_count() {
            2 => self.select_word_at_mouse(),
            3 => self.select_line_at_mouse(),
            _ => self.point_caret(self.input_state.mods.shift),
        }
        false
    }

    /// 正文连续点击计数（1 单击 / 2 双击 / 3 三击）：同点（≤4 逻辑 px）且
    /// 间隔 ≤500ms 递增；三击之后继续连点则重新从单击计起。
    fn click_count(&mut self) -> u8 {
        let now = Instant::now();
        let pos = self.input_state.mouse;
        let slop = CLICK_SLOP * self.input_state.scale as f64;
        let count = match self.window_state.last_click {
            Some((t, p, n))
                if now.duration_since(t) <= CLICK_GAP
                    && (p.x - pos.x).abs() <= slop
                    && (p.y - pos.y).abs() <= slop =>
            {
                if n >= 3 {
                    1
                } else {
                    n + 1
                }
            }
            _ => 1,
        };
        self.window_state.last_click = Some((now, pos, count));
        count
    }

    /// 双击选词：命中偏移处按字符类别选中整词/空白串。
    pub(crate) fn select_word_at_mouse(&mut self) {
        let Some(off) = self.hit_offset() else {
            return;
        };
        if !self.editor.select_word_at(off) {
            return;
        }
        self.window_state.goal_x = None;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 三击选行：命中偏移处选中整行（不含行尾换行）。
    pub(crate) fn select_line_at_mouse(&mut self) {
        let Some(off) = self.hit_offset() else {
            return;
        };
        if !self.editor.select_line_at(off) {
            return;
        }
        self.window_state.goal_x = None;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 悬停高亮（右键菜单项 / 菜单标题 / 弹出菜单项 / 大纲条目）。
    pub(crate) fn update_hover(&mut self) {
        let (x, y) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        if self.ui_state.ctx.open {
            let l = self.ctx_layout();
            let h = ctx_menu::hit(&l, x, y);
            if self.ui_state.ctx.hover != h {
                self.ui_state.ctx.hover = h;
                self.request_redraw();
            }
            self.set_hover_link(None, x, y);
            return; // 右键菜单打开时其余悬停不更新
        }
        if self.ui_state.history.open {
            let l = self.history_layout();
            let count = self.ui_state.history.entries.len();
            let top = history::first_visible(self.ui_state.history.scroll, self.input_state.scale);
            let h = history::hit(&l, top, count, x, y, self.input_state.scale);
            let hover = match h {
                history::Hit::Row(i) => Some(i),
                _ => None,
            };
            let action = match h {
                history::Hit::Action(a) => Some(a),
                _ => None,
            };
            if self.ui_state.history.hover != hover || self.ui_state.history.hover_action != action
            {
                self.ui_state.history.hover = hover;
                self.ui_state.history.hover_action = action;
                self.request_redraw();
            }
            self.ui_state.chrome.hover_menu = None;
            self.ui_state.chrome.hover_item = None;
            self.set_hover_link(None, x, y);
            return; // 历史面板打开：不更新正文/其它面板悬停
        }
        if self.ui_state.chrome.scroll_hover {
            self.ui_state.chrome.scroll_hover = false;
        }
        let menu = chrome::hit_menu_bar(x, y, self.input_state.scale);
        let item = self.ui_state.chrome.open.and_then(|open| {
            let recent = self.recent_labels();
            chrome::hit_popup_item(
                x,
                y,
                open,
                self.input_state.scale,
                &recent,
                self.viewport().w,
            )
            .map(|i| (open, i))
        });
        if self.ui_state.chrome.hover_menu != menu || self.ui_state.chrome.hover_item != item {
            self.ui_state.chrome.hover_menu = menu;
            self.ui_state.chrome.hover_item = item;
            self.request_redraw();
        }
        // 悬停最近文件项时，状态栏显示完整路径；离开后恢复原提示
        let recent_paths = &self.persist_state.recent;
        let is_recent_hover = item.and_then(|(open, i)| {
            let recent = self.recent_labels();
            chrome::action_at(open, i, &recent)
        });
        match is_recent_hover {
            Some(chrome::MenuAction::OpenRecent(idx)) if idx < recent_paths.len() => {
                if self.ui_state.chrome.saved_notice.is_none() {
                    self.ui_state.chrome.saved_notice = Some(self.ui_state.chrome.notice.clone());
                }
                self.ui_state.chrome.notice = recent_paths[idx].display().to_string();
            }
            _ => {
                if let Some(saved) = self.ui_state.chrome.saved_notice.take() {
                    self.ui_state.chrome.notice = saved;
                }
            }
        }
        // 滚动条悬停：与点击命中同一判据（轨道/滑块），拖动中也保持高亮
        let vp = self.viewport();
        let sh_bar = chrome::scrollbar_hit(vp, self.doc_height(), x, y, self.input_state.scale);
        if self.ui_state.chrome.scroll_hover != sh_bar {
            self.ui_state.chrome.scroll_hover = sh_bar;
            self.request_redraw();
        }
        let row = self.outline_rect().and_then(|p| {
            outline::hit_row(
                p,
                self.ui_state.outline.entries.len(),
                self.ui_state.outline.scroll,
                self.input_state.scale,
                x,
                y,
            )
        });
        if self.ui_state.outline.hover != row {
            self.ui_state.outline.hover = row;
            self.request_redraw();
        }
        // 查找栏按钮悬停（栏外与栏内空白均为 None/Bar，仅按钮高亮）
        let sh = self.search_rect().and_then(|l| {
            search::hit(
                &l,
                self.input_state.scale,
                self.ui_state.search.replace_open,
                x,
                y,
            )
        });
        if self.ui_state.search.hover != sh {
            self.ui_state.search.hover = sh;
            self.request_redraw();
        }
        // 快速打开候选项悬停
        let qrow = self.quick_rect().and_then(|l| {
            quick_open::hit(
                &l,
                self.ui_state.quick.top,
                self.ui_state.quick.filtered.len(),
                x,
                y,
            )
        });
        if self.ui_state.quick.hover != qrow {
            self.ui_state.quick.hover = qrow;
            self.request_redraw();
        }
        // 悬停提示（N5）：正文与表格单元格内的 Link 节点显示完整目标 URL；
        // `<abbr title>` 显示 title 文本（同一浮层样式）。模态/拖动（含表格列宽）不显示；
        // 选区覆盖、鼠标移出正文区同样清除。
        let inside = x >= vp.x && x < vp.x + vp.w && y >= vp.y && y < vp.y + vp.h;
        let tip = if !inside
            || self.ui_state.modal.is_some()
            || self.window_state.dragging
            || self.ui_state.tables.drag.is_some()
        {
            None
        } else {
            let off = self.hit_offset();
            off.and_then(|off| {
                let selected = self
                    .editor
                    .selection()
                    .is_some_and(|s| s.start <= off && off < s.end);
                if selected {
                    None
                } else {
                    self.link_at(off)
                        .map(|u| crate::normalize_link_url(&u))
                        .or_else(|| self.abbr_title_at(off))
                }
            })
        };
        self.set_hover_link(tip, x, y);
    }

    /// 写入链接悬停态：URL 未变不重绘（鼠标在链接内移动保留首次锚点，
    /// 浮层位置稳定）；进入/离开/换链接才请求重绘。无定时器。
    fn set_hover_link(&mut self, url: Option<String>, x: f32, y: f32) {
        let same = match (&url, &self.ui_state.hover_link) {
            (None, None) => true,
            (Some(u), Some((cur, _, _))) => u == cur,
            _ => false,
        };
        if same {
            return;
        }
        self.ui_state.hover_link = url.map(|u| (u, x, y));
        self.request_redraw();
    }

    /// 切换大纲面板：版心宽变化等价一次 resize，布局/表格键随宽度量化值失配。
    pub(crate) fn toggle_outline(&mut self) {
        self.ui_state.outline.open = !self.ui_state.outline.open;
        self.ui_state.outline.hover = None;
        self.ui_state.outline.scroll = 0.0;
        // 重新打开时强制按光标同步一次（避免沿用上次的「已跟随」位置）
        self.ui_state.outline.last_caret = None;
        self.refresh_index();
        if self.ui_state.outline.open {
            self.refresh_outline();
        } else {
            self.ui_state.outline.entries.clear();
        }
        self.ensure_layouts_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 切换源码模式（Ctrl+/）：全文按 Markdown 源码排版。
    /// source_mode 进入 base_key → 布局/光栅/表格缓存按因果键整体失配，无需手工清缓存；
    /// 光标与滚动位置不受影响（不动 editor，也不写 scroll_y）。
    pub(crate) fn toggle_source_mode(&mut self) {
        self.input_state.source_mode = !self.input_state.source_mode;
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 打开设置模态（Ctrl+, / 视图菜单 → 设置…）。
    /// 先关掉可能打开的主菜单/右键菜单，避免层叠遮挡。
    pub(crate) fn open_settings(&mut self) {
        // 关掉主菜单、右键菜单（打开设置前清理）
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        // 关掉其他浮层（查找、快速打开）
        self.ui_state.search.open = false;
        self.ui_state.quick.open = false;
        let buttons = crate::view::settings::buttons();
        let form = crate::view::settings::SettingsForm::from_config(&self.config);
        let state = crate::view::settings::SettingsState::new(&form);
        self.ui_state.settings = Some(crate::app::SettingsModal {
            form,
            state,
            layout: None,
            buttons,
        });
        self.request_redraw();
    }

    /// 设置模态结论落地：
    /// - OK（BTN_OK）：把表单值应用到 Config，写 config.toml（ConfigSave::All），刷新 UI（主题/字体/滚动等立即生效）。
    /// - Reset Defaults（BTN_RESET_DEFAULTS）：只在表单内恢复默认（用户还可以 OK/Cancel）。
    /// - Cancel（BTN_CANCEL）：关闭不写盘。
    #[allow(dead_code)]
    pub(crate) fn resolve_settings(&mut self, id: u16) {
        use crate::view::settings::{BTN_CANCEL, BTN_OK, BTN_RESET_DEFAULTS};
        let Some(s) = self.ui_state.settings.as_mut() else {
            return;
        };
        match id {
            BTN_OK => {
                // 应用表单值到 Config
                self.config = s.form.apply(&self.config);
                // 持久化完整 Config；失败时 save_config 已写入可见错误，不得被
                // 后续成功提示覆盖（DEF-T10-008）。
                let saved = self.save_config(ConfigSave::All);
                // 立即刷新所有受影响的 UI（主题模式、字体、平滑滚动等）
                self.refresh_settings_effects();
                if saved {
                    self.ui_state.chrome.notice = t(I18nKey::NoticeSettingsSaved).into();
                }
                self.ui_state.settings = None;
                self.request_redraw();
            }
            BTN_RESET_DEFAULTS => {
                // 表单内恢复默认（不改 Config）：用户还可以 OK 或 Cancel
                s.form = crate::view::settings::SettingsForm::from_config(&Config::default());
                s.state = crate::view::settings::SettingsState::new(&s.form);
                s.layout = None; // 强制在下次 paint 时重算
                self.request_redraw();
            }
            BTN_CANCEL => {
                self.ui_state.settings = None;
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// 设置应用后立即生效：主题模式、字体缓存、平滑滚动。不要求重启。
    ///
    /// 走 `refresh_theme_visuals` 而非只换 `Theme` 值：主题代次必须递增，否则
    /// Dark→System（系统为浅色）时布局/光栅缓存仍按旧深色主题命中，正文保持浅灰。
    #[allow(dead_code)]
    pub(crate) fn refresh_settings_effects(&mut self) {
        // 1) 主题：mode 变了需要重新取系统深色并刷新 Theme 派生值
        if self.config.mode == ThemeMode::System {
            if let Some(dark) = self.ports.system.is_dark() {
                self.input_state.system_dark = dark;
            }
        }
        self.input_state.theme = Theme::of(self.config.mode, self.input_state.system_dark);
        // 2) 平滑滚动：WindowState 独立存储，需要同步
        self.window_state.smooth = self.config.smooth_scroll;
        // 3) 文档字体族/字号/行高/制表符 + 主题配色：统一由主题刷新入口
        //    递增代次、重排可见块并重算几何（base_key 同时含 typography/theme）。
        self.refresh_theme_visuals();
        self.set_ime_area();
        // 成功/失败提示由 resolve_settings 按落盘结果决定，这里不写 notice。
    }

    /// 切换专注模式（阶段 1）：只改视觉开关，不动真值/派生缓存；重绘即生效。
    pub(crate) fn toggle_focus(&mut self) {
        let on = !self.ui_state.writing.focus;
        self.ui_state.writing.focus = on;
        if on {
            // 专注 / 打字机互斥（同为「写作模式」，同开语义混乱）
            self.ui_state.writing.typewriter = false;
        }
        self.ui_state.chrome.notice = tf(
            I18nKey::NoticeFocusMode,
            &[if on {
                t(I18nKey::ToggleOn)
            } else {
                t(I18nKey::ToggleOff)
            }],
        );
        self.request_redraw();
    }

    /// 切换打字机模式（阶段 1）：开启即刻把光标行移到视口锚点；之后每次
    /// `scroll_to_caret`（编辑/垂直移动）都保持该锚点。
    pub(crate) fn toggle_typewriter(&mut self) {
        let on = !self.ui_state.writing.typewriter;
        self.ui_state.writing.typewriter = on;
        if on {
            // 专注 / 打字机互斥
            self.ui_state.writing.focus = false;
            self.scroll_to_caret();
            self.set_ime_area();
        }
        self.ui_state.chrome.notice = tf(
            I18nKey::NoticeTypewriterMode,
            &[if on {
                t(I18nKey::ToggleOn)
            } else {
                t(I18nKey::ToggleOff)
            }],
        );
        self.request_redraw();
    }

    /// 从块索引重建标题条目（仅面板开启时需要）：`Session::heading` 剥除 ATX 与
    /// inline 标记，空标题不生成条目。条目顺序即文档顺序。
    pub(crate) fn refresh_outline(&mut self) {
        let mut entries = Vec::new();
        for g in &self.index.blocks {
            if !matches!(
                g.kind,
                BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
            ) {
                continue;
            }
            let Some((level, text)) = self.editor.session().heading(g.id) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            entries.push(crate::view::OutlineEntry {
                level,
                text,
                block: g.id,
                start: g.range.start,
            });
        }
        let count = entries.len();
        self.ui_state.outline.entries = entries;
        if self.ui_state.outline.hover.is_some_and(|h| h >= count) {
            self.ui_state.outline.hover = None;
        }
    }

    /// `[TOC]` 视图目录的标题输入：按文档顺序返回 (级别, 剥标记文本)。
    /// 与大纲面板同源（`Session::heading`），空标题不入目录。
    pub(crate) fn toc_entries(&self) -> Vec<(u8, String)> {
        let mut out = Vec::new();
        for g in &self.index.blocks {
            if !matches!(
                g.kind,
                BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
            ) {
                continue;
            }
            let Some((level, text)) = self.editor.session().heading(g.id) else {
                continue;
            };
            if !text.is_empty() {
                out.push((level, text));
            }
        }
        out
    }

    /// 大纲面板矩形（关闭时 None）：右缘贴滚动条左缘，纵向占满内容视口。
    pub(crate) fn outline_rect(&self) -> Option<Rect> {
        if !self.ui_state.outline.open {
            return None;
        }
        let vp = self.viewport();
        Some(outline::panel_rect(
            vp.x + vp.w - chrome::SCROLL_W * self.input_state.scale,
            vp.y,
            vp.h,
            self.input_state.scale,
        ))
    }

    /// 点击条目：光标落到标题块起点，并把该块顶滚到视口顶。
    pub(crate) fn jump_outline(&mut self, index: usize) {
        let Some(entry) = self.ui_state.outline.entries.get(index).cloned() else {
            return;
        };
        let top = self
            .index
            .blocks
            .iter()
            .find(|g| g.id == entry.block)
            .map(|g| g.top);
        if !self.editor.set_caret(entry.start) {
            return;
        }
        self.window_state.goal_x = None;
        if self.ui_state.writing.typewriter {
            // 打字机模式下大纲跳转也必须落到锚点（当前行而非视口顶部），
            // 避免首帧停在顶部、要等下一次输入才被纠正（DEF-T11-E1-003）。
            self.window_state.goal_y = self.window_state.scroll_y;
            self.scroll_to_caret();
        } else if let Some(top) = top {
            let vp = self.viewport();
            let max = chrome::max_scroll(vp, self.doc_height());
            let target = (top - MARGIN * self.input_state.scale).clamp(0.0, max);
            self.window_state.goal_y = target;
            if !self.window_state.smooth {
                self.window_state.scroll_y = self.window_state.goal_y;
            }
        }
        self.set_ime_area();
        self.request_redraw();
    }

    /// 滚轮：右键菜单打开时吞掉（瞬态菜单不滚动正文）；快速打开浮层内滚动候选，
    /// 指针在大纲面板内时滚动面板，否则滚动正文。
    pub(crate) fn on_wheel(&mut self, d: f32) {
        // 设置模态滚动的是字段区，不是背景正文（DEF-T07-E1-001）。
        if self.ui_state.settings.is_some() {
            let layout = self.settings_layout();
            if let Some(s) = self.ui_state.settings.as_mut() {
                let step = if d > 0.0 { -1 } else { 1 };
                s.state.scroll(step, layout.max_scroll);
            }
            self.request_redraw();
            return;
        }
        if self.ui_state.modal.is_some() {
            return; // 模态优先：遮罩下不滚动正文
        }
        if self.ui_state.ctx.open {
            return;
        }
        if self.ui_state.quick.open {
            let n = self.ui_state.quick.filtered.len();
            let max = n.saturating_sub(quick_open::MAX_VISIBLE);
            let step = if d > 0.0 { -3 } else { 3 };
            let top = (self.ui_state.quick.top as i32 + step).clamp(0, max as i32) as usize;
            if top != self.ui_state.quick.top {
                self.ui_state.quick.top = top;
                self.request_redraw();
            }
            return;
        }
        if let Some(panel) = self.outline_rect() {
            let (x, y) = (
                self.input_state.mouse.x as f32,
                self.input_state.mouse.y as f32,
            );
            if x >= panel.x && x < panel.x + panel.w && y >= panel.y && y < panel.y + panel.h {
                let max = outline::max_scroll(
                    panel,
                    self.ui_state.outline.entries.len(),
                    self.input_state.scale,
                );
                self.ui_state.outline.scroll = (self.ui_state.outline.scroll - d).clamp(0.0, max);
                self.request_redraw();
                return;
            }
        }
        if self.ui_state.history.open {
            let panel = history::panel_rect(
                Rect {
                    x: self.viewport().x,
                    y: self.viewport().y,
                    w: self.viewport().w,
                    h: self.viewport().h,
                },
                self.input_state.scale,
            );
            let (x, y) = (
                self.input_state.mouse.x as f32,
                self.input_state.mouse.y as f32,
            );
            if x >= panel.x && x < panel.x + panel.w && y >= panel.y && y < panel.y + panel.h {
                let max = history::max_scroll(
                    panel,
                    self.ui_state.history.entries.len(),
                    self.input_state.scale,
                );
                self.ui_state.history.scroll = (self.ui_state.history.scroll - d).clamp(0.0, max);
                self.request_redraw();
                return;
            }
        }
        let vp = self.viewport();
        // 正文滚动：光标位置不变，但用户已开始操作文档，隐藏半周期应被打断。
        self.note_caret_activity();
        let max = chrome::max_scroll(vp, self.doc_height());
        // B3：写入 goal_y；smooth=false 时 animate() 首帧 goal_y==scroll_y 零开销同步
        self.window_state.goal_y = (self.window_state.goal_y - d).clamp(0.0, max);
        if !self.window_state.smooth {
            self.window_state.scroll_y = self.window_state.goal_y;
        }
        self.set_ime_area();
        self.request_redraw();
    }

    // ── 快速打开（阶段 1）：浮层候选、只读过滤与打开 ───────────────────────────

    /// 浮层矩形（关闭时 None）：内容视口内水平居中。
    pub(crate) fn quick_rect(&self) -> Option<quick_open::Layout> {
        if !self.ui_state.quick.open {
            return None;
        }
        let vp = self.viewport();
        Some(quick_open::layout(
            vp.x,
            vp.y,
            vp.w,
            vp.h,
            self.input_state.scale,
            self.ui_state.quick.filtered.len(),
        ))
    }

    /// 打开浮层（Ctrl+P / 菜单）：关闭菜单与查找栏，重建候选（最近文件 + 当前目录）。
    pub(crate) fn open_quick(&mut self) {
        self.close_history(); // 浮层互斥：任何时刻只有一个导航面板
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        if self.ui_state.search.open {
            self.ui_state.search.open = false;
            self.ui_state.search.replace_open = false;
            self.ui_state.search.clear();
        }
        self.editor.clear_preedit();
        self.ui_state.quick.open = true;
        self.refresh_quick();
    }

    /// 重建候选：最近文件（去重）在前，当前文档所在目录的 Markdown 文件其后。
    /// 无关联路径的草稿只有最近文件；目录读取失败按空处理（浮层仍可用）。
    pub(crate) fn refresh_quick(&mut self) {
        let dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.to_path_buf());
        let files = dir
            .as_ref()
            .and_then(|d| self.ports.store.list_files(d).ok())
            .unwrap_or_default();
        self.ui_state.quick.items = quick_open::candidates(
            &self.persist_state.recent,
            &files,
            quick_open::CANDIDATE_CAP,
        );
        self.ui_state.quick.refilter();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 关闭浮层（Esc / 点击面板外）：清空查询与候选。
    pub(crate) fn close_quick(&mut self) {
        if !self.ui_state.quick.open {
            return;
        }
        self.ui_state.quick.open = false;
        self.ui_state.quick.clear();
        self.editor.clear_preedit();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 快速打开 toggle（Ctrl+P）：已开则关，未开则开。
    pub(crate) fn toggle_quick(&mut self) {
        if self.ui_state.quick.open {
            self.close_quick();
        } else {
            self.open_quick();
        }
    }

    /// 按 Esc 优先级关闭面板：快速打开 > 替换栏 > 查找栏 > 大纲面板 > 源码模式。
    /// 无面板打开时关闭菜单和右键菜单（兜底）。全屏不受 Esc 影响。
    pub(crate) fn close_top_panel(&mut self) {
        if self.ui_state.quick.open {
            self.close_quick();
            return;
        }
        if self.ui_state.search.open && self.ui_state.search.replace_open {
            self.close_search();
            return;
        }
        if self.ui_state.search.open {
            self.close_search();
            return;
        }
        if self.ui_state.outline.open {
            self.toggle_outline();
            return;
        }
        if self.input_state.source_mode {
            self.toggle_source_mode();
            return;
        }
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        self.request_redraw();
    }

    /// 查找栏 toggle（Ctrl+F）：已开则关，未开则开。
    pub(crate) fn toggle_search(&mut self) {
        if self.ui_state.search.open {
            self.close_search();
        } else {
            self.open_search();
        }
    }

    /// 替换栏 toggle（Ctrl+H）：已开 replace_open 则关整个 search，否则打开替换。
    pub(crate) fn toggle_replace(&mut self) {
        if self.ui_state.search.open && self.ui_state.search.replace_open {
            self.close_search();
        } else {
            self.open_replace();
        }
    }

    /// 过滤串追加（含 IME 提交）：重算候选并回到首项。
    pub(crate) fn quick_input(&mut self, text: &str) {
        if !self.ui_state.quick.open || text.is_empty() {
            return;
        }
        self.ui_state.quick.query.push_str(text);
        self.ui_state.quick.refilter();
        self.set_ime_area();
        self.request_redraw();
    }

    pub(crate) fn quick_backspace(&mut self) {
        if !self.ui_state.quick.open {
            return;
        }
        if self.ui_state.quick.query.pop().is_some() {
            self.ui_state.quick.refilter();
            self.set_ime_area();
            self.request_redraw();
        }
    }

    pub(crate) fn quick_step(&mut self, delta: i32) {
        if !self.ui_state.quick.open {
            return;
        }
        self.ui_state.quick.step(delta);
        self.request_redraw();
    }

    /// Home：选中首个候选。
    pub(crate) fn quick_first(&mut self) {
        self.quick_select(0);
    }

    /// End：选中末个候选（空列表时归零）。
    pub(crate) fn quick_last(&mut self) {
        let last = self.ui_state.quick.filtered.len().saturating_sub(1);
        self.quick_select(last);
    }

    /// PageUp/PageDown：按面板可见行数翻页。
    pub(crate) fn quick_page(&mut self, direction: i32) {
        let step = quick_open::MAX_VISIBLE as i32;
        self.quick_step(direction * step);
    }

    /// 直接定位选中项（钳制在 `filtered` 范围内并同步滚动）。
    fn quick_select(&mut self, selected: usize) {
        if !self.ui_state.quick.open {
            return;
        }
        let n = self.ui_state.quick.filtered.len();
        let quick = &mut self.ui_state.quick;
        quick.selected = if n == 0 { 0 } else { selected.min(n - 1) };
        quick.ensure_visible();
        self.request_redraw();
    }

    /// 回车/点击：打开选中项；脏文档由 open_path 拒绝并在状态栏提示。
    pub(crate) fn quick_accept(&mut self) {
        let Some(path) = self.ui_state.quick.current().map(|it| it.path.clone()) else {
            return;
        };
        self.close_quick();
        self.open_path(path);
    }

    // ── 文档内查找（P3-3）：浮栏状态、只读输入与命中跳转 ─────────────────────

    /// 查找栏矩形（关闭时 None）：贴内容视口右上角，右缘让出滚动条与大纲面板。
    pub(crate) fn search_rect(&self) -> Option<search::BarLayout> {
        if !self.ui_state.search.open {
            return None;
        }
        let vp = self.viewport();
        let panel = if self.ui_state.outline.open {
            outline::PANEL_W * self.input_state.scale
        } else {
            0.0
        };
        let right = vp.x + vp.w - chrome::SCROLL_W * self.input_state.scale - panel;
        Some(search::layout(right, vp.y, self.input_state.scale))
    }

    /// 打开查找栏（Ctrl+F / 菜单）：查询清空、文档转只读。已打开时仅重申查询焦点。
    pub(crate) fn open_search(&mut self) {
        if self.ui_state.search.open {
            self.ui_state.search.focus = search::Field::Query;
            self.set_ime_area();
            self.request_redraw();
            return;
        }
        self.close_history(); // 浮层互斥：任何时刻只有一个导航面板
        self.close_quick();
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        self.editor.clear_preedit();
        self.ui_state.search.open = true;
        self.ui_state.search.replace_open = false;
        self.ui_state.search.clear();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 打开查找替换栏（Ctrl+H / 菜单）：首次打开焦点落在查询串（先输词再 Tab 到替换，
    /// 避免首字进替换框导致「无命中、看似无响应」）；栏已开时只展开替换行并聚焦替换串。
    pub(crate) fn open_replace(&mut self) {
        self.close_history(); // 浮层互斥：任何时刻只有一个导航面板
        self.close_quick();
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        if self.ui_state.search.open {
            self.ui_state.search.replace_open = true;
            self.ui_state.search.focus = search::Field::Replace;
        } else {
            self.ui_state.search.open = true;
            self.ui_state.search.replace_open = true;
            self.ui_state.search.clear();
            self.ui_state.search.focus = search::Field::Query;
        }
        self.set_ime_area();
        self.request_redraw();
    }

    /// 关闭查找栏（Esc / ×）：查询与替换串清空、高亮消失；光标停在当前命中处不动。
    pub(crate) fn close_search(&mut self) {
        if !self.ui_state.search.open {
            return;
        }
        self.ui_state.search.open = false;
        self.ui_state.search.replace_open = false;
        self.ui_state.search.clear();
        self.editor.clear_preedit();
        self.set_ime_area();
        self.request_redraw();
    }

    /// Tab 在查询/替换两个输入框之间切换焦点（仅替换模式）。
    pub(crate) fn search_toggle_field(&mut self) {
        if !self.ui_state.search.open || !self.ui_state.search.replace_open {
            return;
        }
        self.ui_state.search.focus = match self.ui_state.search.focus {
            search::Field::Query => search::Field::Replace,
            search::Field::Replace => search::Field::Query,
        };
        self.set_ime_area();
        self.request_redraw();
    }

    /// 「区分大小写」开关：翻转后立即重扫（n/m 与光标同步），会话内保持。
    pub(crate) fn search_toggle_case(&mut self) {
        if !self.ui_state.search.open {
            return;
        }
        self.ui_state.search.case_sensitive = !self.ui_state.search.case_sensitive;
        self.search_rescan();
        self.set_ime_area();
        self.request_redraw();
    }

    /// Enter：查询框内 = 下一个命中；替换框内 = 替换当前并前进到下一处
    /// （复用 `replace_current`），Ctrl+Enter = 全部替换（复用 `replace_all`）。
    fn search_enter(&mut self) {
        if self.ui_state.search.focus != search::Field::Replace {
            self.search_next();
            return;
        }
        if self.input_state.mods.ctrl {
            self.replace_all();
            return;
        }
        let Some(m) = self.ui_state.search.current_match() else {
            return;
        };
        // 替换串落盘后长度可能变化：以「旧起点 + 新串字节长度」为下一处起点。
        let from = m.start + self.ui_state.search.replace.len();
        self.replace_current();
        self.ui_state.search.select_after(from);
        self.jump_to_match();
    }

    /// 焦点框追加文本（含 IME 提交）：查询变化才重算匹配并跳首个。
    pub(crate) fn search_input(&mut self, text: &str) {
        if !self.ui_state.search.open || text.is_empty() {
            return;
        }
        match self.ui_state.search.focus {
            search::Field::Query => {
                self.ui_state.search.query.push_str(text);
                self.search_rescan();
            }
            search::Field::Replace => {
                self.ui_state.search.replace.push_str(text);
                self.request_redraw();
            }
        }
    }

    /// 焦点框退格：仅查询变化才重算并跳首个。
    pub(crate) fn search_backspace(&mut self) {
        if !self.ui_state.search.open {
            return;
        }
        match self.ui_state.search.focus {
            search::Field::Query => {
                if self.ui_state.search.query.pop().is_some() {
                    self.search_rescan();
                }
            }
            search::Field::Replace => {
                if self.ui_state.search.replace.pop().is_some() {
                    self.request_redraw();
                }
            }
        }
    }

    /// 查找栏内粘贴（Ctrl+V）：只取剪贴板纯文本，剥掉换行变成单行，
    /// 文件列表/图片在查找栏语境下一律当作无效（查询框不接受 markdown）。
    pub(crate) fn search_paste(&mut self) {
        if !self.ui_state.search.open {
            return;
        }
        let Some(raw) = self.ports.clipboard.get() else {
            return;
        };
        if raw.is_empty() {
            return;
        }
        // 单行化：\r\n / \n / \r 统一换成空格；过长直接丢弃后半段（查询框几百字够用）
        const MAX: usize = 500;
        let single: String = raw
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .take(MAX)
            .collect();
        if single.is_empty() {
            return;
        }
        match self.ui_state.search.focus {
            search::Field::Query => {
                self.ui_state.search.query.push_str(&single);
                self.search_rescan();
            }
            search::Field::Replace => {
                self.ui_state.search.replace.push_str(&single);
                self.request_redraw();
            }
        }
    }

    /// 下一个命中（循环）。
    pub(crate) fn search_next(&mut self) {
        if self.ui_state.search.step(true).is_some() {
            self.jump_to_match();
        }
    }

    /// 上一个命中（循环）。
    pub(crate) fn search_prev(&mut self) {
        if self.ui_state.search.step(false).is_some() {
            self.jump_to_match();
        }
    }

    /// Home：跳到首个命中（查询框无内部光标，故按命中导航）。
    pub(crate) fn search_first_match(&mut self) {
        if self.ui_state.search.matches.is_empty() {
            return;
        }
        self.ui_state.search.current = 0;
        self.jump_to_match();
    }

    /// End：跳到末个命中。
    pub(crate) fn search_last_match(&mut self) {
        let Some(last) = self.ui_state.search.matches.len().checked_sub(1) else {
            return;
        };
        self.ui_state.search.current = last;
        self.jump_to_match();
    }

    /// 单次替换：替换当前命中（一步事务），重扫后跳到剩余的首个命中。
    pub(crate) fn replace_current(&mut self) {
        if !self.ui_state.search.open || !self.ui_state.search.replace_open {
            return;
        }
        let Some(m) = self.ui_state.search.current_match() else {
            return;
        };
        let text = self.ui_state.search.replace.clone();
        let Some(dirty) = self.editor.replace_range(m, &text) else {
            return;
        };
        self.window_state.goal_x = None;
        self.after_edit(dirty);
        self.search_rescan(); // 命中缓存重建：后续替换绝不基于旧区间
    }

    /// 全部替换：一次事务覆盖所有命中（单撤销步），随后重建命中缓存。
    pub(crate) fn replace_all(&mut self) {
        if !self.ui_state.search.open || !self.ui_state.search.replace_open {
            return;
        }
        let matches = self.ui_state.search.matches.clone();
        let text = self.ui_state.search.replace.clone();
        let Some((dirty, _)) = self.editor.replace_all(&matches, &text) else {
            return;
        };
        self.window_state.goal_x = None;
        self.after_edit(dirty);
        self.search_rescan();
    }

    /// 全文重扫：查询变化或替换落盘后执行，随后跳到当前命中。
    fn search_rescan(&mut self) {
        let n = self.editor.session().text_len();
        let hay = self
            .editor
            .session()
            .text_slice(veil_core::Range::new(0, n))
            .unwrap_or_default();
        self.ui_state.search.rescan(&hay);
        self.jump_to_match();
    }

    /// 光标落到当前命中起点并滚动入视口（纯视图移动，不置脏）。
    fn jump_to_match(&mut self) {
        let Some(m) = self.ui_state.search.current_match() else {
            self.set_ime_area();
            self.request_redraw(); // 无命中亦须刷新计数标签
            return;
        };
        if !self.editor.set_caret(m.start) {
            return;
        }
        // B3：命中位于折叠 details 内 → 自动展开（块级组或行内 details），
        // 否则命中高亮画不出来、光标落在隐藏区。
        self.sync_details_at_caret();
        self.window_state.goal_x = None;
        self.after_edit(Vec::new()); // 补齐可见/光标块布局并滚动入视口
        self.set_ime_area();
    }
}

/// 空 mermaid 块判定：body trim 后为空，或所有非空行都以 `%%` 开头（纯注释块）。
/// 与 `view/layout.rs::fenced_subs` 的提示行判定保持同口径。
fn mermaid_body_is_empty(body: &str) -> bool {
    body.lines()
        .map(str::trim)
        .all(|l| l.is_empty() || l.starts_with("%%"))
}

/// 解析脚注定义块首行 `[^id]: ` 的 id。
/// 为什么在这里重写一遍：`layout::footnote_def_prefix` 是排版私有实现，
/// 交互层只借它定位命中，不应把排版模块的内部约定变成公共 API。
/// 规则与 core `parse_footnote_def` 同口径（缩进 ≤3、字母数字/-/_、`]:`）。
fn footnote_def_id(text: &str) -> Option<&str> {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let bytes = text.as_bytes();
    if indent > 3 || bytes.get(indent) != Some(&b'[') || bytes.get(indent + 1) != Some(&b'^') {
        return None;
    }
    let start = indent + 2;
    let mut i = start;
    loop {
        let ch = text.get(i..)?.chars().next()?;
        if ch == ']' {
            break;
        }
        if !(ch.is_alphanumeric() || ch == '-' || ch == '_') {
            return None;
        }
        i += ch.len_utf8();
    }
    if i == start || bytes.get(i + 1) != Some(&b':') {
        return None;
    }
    Some(&text[start..i])
}

/// 图片链接目标（2f）：文档目录内取相对路径，目录外取绝对路径；
/// 统一 / 分隔符（Markdown 链接语义），含空格加 <> 定界。
fn image_link_target(doc: &Path, img: &Path) -> String {
    let base = doc.parent().unwrap_or_else(|| Path::new(""));
    let rel = img.strip_prefix(base).unwrap_or(img);
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.contains(' ') {
        format!("<{s}>")
    } else {
        s
    }
}

/// winit 键 → 内部 `Key`。
///
/// 关键约束（K1 二次修复）：Windows 上 `event.text` 是**剥离 Ctrl 后**的文本
/// （Ctrl+S 的 text 就是 `"s"`），所以 Ctrl 按下时绝不能走文本分支，否则全部
/// Ctrl 组合都会变成 `Key::Text("s")` → `translate` 返回 None → 快捷键静默失效。
/// Ctrl 组合一律交给 `key_without_modifiers()`（还原字母）；AltGr（Ctrl+Alt）
/// 仍需走文本，否则欧洲布局字符无法上屏。
pub(crate) fn map_key(event: &winit::event::KeyEvent, mods: Mods) -> Option<Key> {
    let ctrl = mods.ctrl && !mods.alt;
    if ctrl {
        if let Some(key) = ctrl_punctuation(&event.logical_key, event.physical_key) {
            return Some(key);
        }
    }
    map_key_parts(event.text.as_deref(), event.key_without_modifiers(), mods)
}

/// Ctrl 组合下的标点快捷键：`/`（源码模式）、`,`（设置）与小键盘缩放键。
/// 逻辑键与物理键都认：部分布局/IME 只给其中之一（如在 Windows 中文输入法下
/// 逻辑键可能不是字符键）。仅 Ctrl 组合调用，普通标点仍走文本分支，不被吞。
/// 小键盘 `+`/`-`/`0` 在 NumLock 开/关时逻辑键可能是 `+`/`-`/`0` 或导航键，
/// 故同时按物理键识别，缩放语义与主键盘 `Ctrl+=/-/0` 一致（DEF-T06-004）。
pub(crate) fn ctrl_punctuation(logical: &WKey, physical: PhysicalKey) -> Option<Key> {
    let logical = match logical {
        WKey::Character(c) => Some(c.as_str()),
        _ => None,
    };
    if logical == Some("/") || physical == PhysicalKey::Code(KeyCode::Slash) {
        return Some(Key::Slash);
    }
    if logical == Some(",") || physical == PhysicalKey::Code(KeyCode::Comma) {
        return Some(Key::Comma);
    }
    if logical == Some("+") || physical == PhysicalKey::Code(KeyCode::NumpadAdd) {
        return Some(Key::Equal);
    }
    if physical == PhysicalKey::Code(KeyCode::NumpadSubtract) {
        return Some(Key::Minus);
    }
    if physical == PhysicalKey::Code(KeyCode::Numpad0) {
        return Some(Key::Digit0);
    }
    None
}

/// `map_key` 的纯函数内核：文本 / 无修饰键 / 修饰键三份输入，可穷举测试。
pub(crate) fn map_key_parts(text: Option<&str>, wom: WKey, mods: Mods) -> Option<Key> {
    let ctrl = mods.ctrl && !mods.alt;
    if !ctrl {
        if let Some(t) = text {
            if !t.is_empty() && !t.chars().any(|c| c.is_control()) {
                return Some(Key::Text(t.to_string()));
            }
        }
    }
    match wom {
        WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
        WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
        WKey::Named(NamedKey::ArrowUp) => Some(Key::Up),
        WKey::Named(NamedKey::ArrowDown) => Some(Key::Down),
        WKey::Named(NamedKey::Home) => Some(Key::Home),
        WKey::Named(NamedKey::End) => Some(Key::End),
        WKey::Named(NamedKey::PageUp) => Some(Key::PageUp),
        WKey::Named(NamedKey::PageDown) => Some(Key::PageDown),
        WKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
        WKey::Named(NamedKey::Delete) => Some(Key::Delete),
        WKey::Named(NamedKey::Enter) => Some(Key::Enter),
        WKey::Named(NamedKey::Tab) => Some(Key::Tab),
        WKey::Named(NamedKey::Escape) => Some(Key::Escape),
        WKey::Named(NamedKey::F11) => Some(Key::F11),
        WKey::Named(NamedKey::Space) => Some(Key::Text(" ".into())),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("z") => Some(Key::Z),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("y") => Some(Key::Y),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("c") => Some(Key::C),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("x") => Some(Key::X),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("v") => Some(Key::V),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("s") => Some(Key::S),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("o") => Some(Key::O),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("e") => Some(Key::E),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("f") => Some(Key::F),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("h") => Some(Key::H),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("a") => Some(Key::A),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("n") => Some(Key::N),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("p") => Some(Key::P),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("r") => Some(Key::R),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("b") => Some(Key::B),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("i") => Some(Key::I),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("k") => Some(Key::K),
        // Ctrl+0/-/= 缩放（3c-2）：无 ctrl 时这些字符的文本由上方 Text 分支承载
        WKey::Character(c) if c.as_str() == "0" => Some(Key::Digit0),
        WKey::Character(c) if c.as_str() == "-" => Some(Key::Minus),
        WKey::Character(c) if c.as_str() == "=" => Some(Key::Equal),
        _ => None,
    }
}

// ── 几何编排（原 geometry.rs）：视口区间/块高/光标坐标/Reveal 指纹 ────────────

impl App {
    /// 版心可用宽（非打印路径）：窗口宽扣除两侧边距、滚动条、大纲与版本历史面板。
    fn available_width(&self) -> f32 {
        let (w, _) = self.win_size();
        let scale = self.input_state.scale;
        let panel = if self.ui_state.outline.open {
            outline::PANEL_W * scale
        } else {
            0.0
        };
        let left = if self.ui_state.history.open {
            history::PANEL_W * scale
        } else {
            0.0
        };
        (w as f32 - 2.0 * MARGIN * scale - chrome::SCROLL_W * scale - panel - left).max(32.0)
    }

    pub(crate) fn content_width(&self) -> f32 {
        let (w, _) = self.win_size();
        let scale = self.input_state.scale;
        // 打印：无滚动条/大纲面板，版心 = 页宽 - 两侧页边距（3b），不受可读列宽上限影响
        if self.window_state.print.is_some() {
            return (w as f32 - 2.0 * MARGIN * scale).max(32.0);
        }
        let available = self.available_width();
        // 可读列宽上限只在超宽窗口启用；窄窗口与改动前逐字节一致
        if w as f32 > WIDE_WINDOW * scale {
            available.min(READABLE_COLS * BASE_FONT_SIZE * scale)
        } else {
            available
        }
    }

    /// 文档内容坐标系横向原点（物理像素）：打印为左边距；
    /// 屏幕语义下超宽窗口把版心居中，窄窗口等于左边距。
    pub(crate) fn content_x(&self) -> f32 {
        let scale = self.input_state.scale;
        if self.window_state.print.is_some() {
            return MARGIN * scale;
        }
        MARGIN * scale
            + ((self.available_width() - self.content_width()) / 2.0).max(0.0)
            + if self.ui_state.history.open {
                history::PANEL_W * scale
            } else {
                0.0
            }
    }

    /// 容器内表格的水平缩进（物理像素）：引用每层一级、列表按内容列。
    /// 与正文同源缩放：外层引用条在版心左侧 8px，表格内容随层右移。
    pub(crate) fn table_indent(&self, container: BlockContainer) -> f32 {
        let scale = self.input_state.scale;
        (f32::from(container.quote_depth) * crate::tokens::doc::QUOTE_STEP
            + f32::from(container.list_indent) * crate::tokens::doc::LIST_STEP)
            * scale
    }

    /// 行内图片可用的最大宽度（B4）：版心宽扣除容器内边距
    /// （引用/列表缩进 + details 卡片内边距），至少 32px。
    pub(crate) fn image_max_width(&self, container: BlockContainer, width: f32) -> f32 {
        let pad = self.table_indent(container)
            + if container.details {
                crate::tokens::doc::DETAILS_PAD * self.input_state.scale
            } else {
                0.0
            };
        (width - pad).max(32.0)
    }

    /// 独占一行图片的高度上限（物理像素）：窗口可视高度 × 80%。
    /// 与版心宽上限一起由 `fit_size_box` 取较小比例，从而「等比适配、不溢出视口」。
    /// 该值是布局输入（缓存键 `BaseKey::img_h_q` 收录其整数像素值），
    /// 因此窗口高度变化会让旧布局自动失配重排。
    pub(crate) fn image_max_height(&self) -> f32 {
        (self.viewport().h * crate::tokens::doc::IMG_STANDALONE_VIEWPORT_RATIO).max(1.0)
    }

    /// 是否因祖先 details 折叠而不可见（B3）。几何重算期间块表被临时取走，
    /// 此时回退到预计算的 `details_hidden`；其余时候沿归属链实时判定，
    /// 保证视图态在几何重算之外被直接修改（测试/交互）也立即正确。
    pub(crate) fn block_hidden(&self, g: &BlockGeom) -> bool {
        if self.window_state.print.is_some() {
            return false;
        }
        if g.container.details_depth == 0 {
            return false;
        }
        if self.index.blocks.is_empty() {
            return g.details_hidden;
        }
        let mut parent = g.details_parent;
        if parent.is_none() {
            let want = g.container.details_depth - 1;
            parent = self
                .index
                .blocks
                .iter()
                .rev()
                .filter(|b| b.range.start < g.range.start)
                .find(|b| {
                    matches!(b.kind, BlockKind::Details { .. }) && b.container.details_depth == want
                })
                .map(|b| b.id);
        }
        while let Some(id) = parent {
            if !self.ui_state.expanded_details.contains(&id) {
                return true;
            }
            parent = self
                .index
                .blocks
                .iter()
                .find(|b| b.id == id)
                .and_then(|h| h.details_parent);
        }
        false
    }

    /// details 卡片需额外覆盖的子块高度（B3：所有层级更深的后续块）。
    /// 按块高 + 与几何一致的段距累加（隐藏子块高度 0、组内段距 0），
    /// 不依赖子块 `top` 是否已重算，视图态变化后可立即取到正确高度。
    pub(crate) fn details_children_height(&self, g: &BlockGeom) -> f32 {
        let Some(i) = self.index.blocks.iter().position(|b| b.id == g.id) else {
            return 0.0;
        };
        let scale = self.input_state.scale;
        let mut total = 0.0;
        // 首子块与头块之间同样有段距（见 gap_between 的头块→首子块分支）。
        let mut prev: Option<&BlockGeom> = Some(g);
        for next in &self.index.blocks[i + 1..] {
            if next.container.details_depth <= g.container.details_depth {
                break;
            }
            if self.block_hidden(next) {
                continue; // 折叠子块整组 0 高，也不贡献段距
            }
            if let Some(p) = prev {
                let blank = self.blank_lines_in_gap(p.range, next.range);
                total += gap_between(
                    &p.kind,
                    p.range,
                    p.container,
                    &next.kind,
                    next.range,
                    next.container,
                    blank,
                    self.engine.doc_line_height(),
                ) * scale;
            }
            total += self.block_height(next);
            prev = Some(next);
        }
        total
    }

    /// 文档总高（含首尾外边距），滚动条与滚轮钳位依据。
    pub(crate) fn doc_height(&self) -> f32 {
        if self.index.blocks.is_empty() {
            2.0 * MARGIN * self.input_state.scale
        } else {
            self.index.total_h
        }
    }

    /// 文本布局缺失（键失配或视口外未排版）时回退廉价估算高度。
    /// 数学/表格分别取渲染图与网格高度（键校验与绘制同源）。
    pub(crate) fn block_height(&self, g: &BlockGeom) -> f32 {
        // B3：折叠 details 的隐藏子块不渲染、不占高度（不是光栅裁剪）。
        if self.block_hidden(g) {
            return 0.0;
        }
        // Details 头块折叠态：只显示 summary 单行 + ▼/▶ 箭头。折叠态是纯 UI 状态，
        // 不参与 layout 缓存 key；直接在 block_height 层面钳高度，避免缓存污染。
        // 打印态恒展开（用户想看完整内容），跳过折叠钳制。
        if let BlockKind::Details { .. } = &g.kind {
            if self.window_state.print.is_none()
                && !self.ui_state.expanded_details.contains(&g.id)
                // 光标进入头块时按 WYSIWYM 显示源码（多行），不做单行裁剪。
                && !crate::view::node_active(g.range, self.effective_caret())
            {
                // 物理像素：doc_line_height 是逻辑像素基准，几何与文本光栅都乘 scale
                return (self.engine.doc_line_height() * self.input_state.scale).round();
            }
        }
        if g.kind == BlockKind::Math && self.math_shows_image(g.id, &g.range) {
            return self
                .cache
                .math(g.id, self.math_key(g.id))
                .map_or(g.est_h, |m| m.height as f32);
        }
        if g.kind == BlockKind::Table && self.table_shows_grid(g.id, &g.range) {
            return self
                .cache
                .table(g.id, self.table_key(g.id))
                .map_or(g.est_h, |t| t.height);
        }
        self.cache
            .layout(
                g.id,
                &g.kind,
                g.range,
                self.base_key(),
                self.effective_caret(),
            )
            .map_or(g.est_h, |l| l.height)
    }

    /// 每视觉行可容纳的字符数（按平均字宽 0.5em 估算，仅用于便宜高度）。
    pub(crate) fn chars_per_line(&self) -> f32 {
        (self.content_width() / (0.5 * BASE_FONT_SIZE * self.input_state.scale)).max(1.0)
    }

    /// 光标所在块索引；文末光标可能落在最后一块的尾换行之后，回退末块。
    pub(crate) fn caret_block_index(&self) -> Option<usize> {
        let caret = self.editor.caret();
        self.index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
            .or_else(|| {
                if caret >= self.editor.session().text_len() {
                    self.index.blocks.len().checked_sub(1)
                } else {
                    None
                }
            })
    }

    /// 光标前紧邻的尾换行个数（0/1/≥2，按字素回退，多字节字符安全）。
    /// 1 = 同段软换行；≥2 = 段落分隔（markdown 折叠多余空行时的空段落）。
    /// 上限 64：仅用于光标锚定与空行高度累计，避免病态输入逐字素回溯。
    fn trailing_newlines(&self, caret: usize) -> usize {
        let mut n = 0;
        let mut c = caret;
        while n < 64 {
            let Some(p) = self.editor.session().prev_grapheme(c) else {
                break;
            };
            // `\r\n` 按 UAX#29 是单个字素簇：CRLF 文档的回车也必须计入尾换行，
            // 否则光标会落入 caret_xy 的行尾钳制、贴在上一行（I1）。
            if matches!(
                self.editor
                    .session()
                    .text_slice(Range::new(p, c))
                    .as_deref(),
                Some("\n" | "\r\n")
            ) {
                n += 1;
                c = p;
            } else {
                break;
            }
        }
        n
    }

    /// 光标屏幕矩形（物理像素，含菜单栏偏移）。(x, y, h)；找不到所在块（空文档）时落视口原点。
    pub(crate) fn caret_screen(&mut self) -> (f32, f32, f32) {
        let caret = self.editor.caret();
        let vp = self.viewport();
        let scale = self.input_state.scale;
        let margin = self.content_x();
        let line_h = BASE_LINE_HEIGHT * scale;
        let scroll = self.window_state.scroll_y;
        // 严格包含（不用文末回退），先分流「光标在块尾换行之后」的情形
        if let Some(i) = self
            .index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
        {
            let (id, start, end, top, is_table) = {
                let g = &self.index.blocks[i];
                (
                    g.id,
                    g.range.start,
                    g.range.end,
                    g.top,
                    g.kind == BlockKind::Table,
                )
            };
            // 网格态表格：源码排版坐标与网格几何不一致，光标改画在活动单元格内容起点
            // （与点击落点、Tab 落点同源，避免「悬空光标」）。
            if is_table
                && self.caret_in_table_cell(id)
                && self.table_shows_grid(id, &Range::new(start, end))
            {
                if let Some((x, y, h)) = self.table_caret_xy(id, top, caret) {
                    return (x, y, h);
                }
            }
            // Enter 后新行尚未成块：布局里没有对应视觉行，钳回末字形会让光标/preedit/
            // IME 候选框贴在上一段末尾（真机缺陷 ①③）。改为锚到文本将出现的位置：
            // 1 个尾换行 = 同段下一视觉行；≥2 = 新段顶边（块底 + 段距）。
            if caret == end && caret > start && self.trailing_newlines(caret) > 0 {
                let nl = self.trailing_newlines(caret);
                let kind = self.index.blocks[i].kind.clone();
                self.ensure_block(id);
                if let Some(l) = self.cache.layouts.get(&id) {
                    // 空行高度（B5）：Enter×1 = 同段下一视觉行（块底）；
                    // Enter×n（n≥2）= 块底 + 段距 + (n-1) 个满行高，与
                    // `gap_between` 的空行累计一致，提交前后光标不跳变。
                    let y = if nl == 1 {
                        top + l.height
                    } else {
                        top + l.height
                            + (gap_after(&kind) + (nl - 1) as f32 * self.engine.doc_line_height())
                                * scale
                    };
                    return (margin, vp.y + y - scroll, line_h);
                }
            }
            // 视口外块可能尚未布局：光标路径按需补齐
            self.ensure_block(id);
            if let Some(l) = self.cache.layouts.get(&id) {
                let (x, y, h) = self.engine.caret_xy(l, caret - start);
                return (margin + x, vp.y + top + y - scroll, h);
            }
        }
        // 块间空隙（Enter 产生的空段尚未成块）：锚到下一块顶边 = 新段将出现的位置；
        // 文末无下一块时锚到末块底 + 段距。旧行为落视口原点，IME 候选框停在左上角。
        if let Some(next) = self.index.blocks.iter().find(|g| g.range.start >= caret) {
            let top = next.top;
            return (margin, vp.y + top - scroll, line_h);
        }
        if let Some(last) = self.index.blocks.last() {
            let top = last.top;
            let h = self.block_height(last);
            let gap = gap_after(&last.kind);
            // 末块之后仍有尾换行（块区间不含尾部空行）：每个空行占 1 行高（B5）。
            let blank = self.blank_lines_in_gap(last.range, Range::new(caret, caret)) as f32;
            return (
                margin,
                vp.y + top + h + (gap + blank * self.engine.doc_line_height()) * scale - scroll,
                line_h,
            );
        }
        (margin, vp.y + margin - scroll, line_h)
    }
}
