//! 交互策略：按键到意图的翻译、编辑后的几何刷新编排、光标移动/命中/点选/
//! 拖选、剪贴板、菜单动作。只发意图与调几何，不做像素绘制。

use crate::app::{App, ConfigSave};
use crate::chrome;
use crate::document::block_index::{gap_after, gap_between, BlockGeom};
use crate::editor::Intent;
use crate::geometry::{Rect, BASE_FONT_SIZE, BASE_LINE_HEIGHT, MARGIN, READABLE_COLS, WIDE_WINDOW};
use crate::html::html_to_markdown;
use crate::i18n::{clip_error, t, tf, Key as I18nKey};
use crate::input::{Action, Key, Mods};
use crate::mode::{route, Cmd, Mode, TextSink};
use crate::theme::{Config, Theme, ThemeMode};
use crate::view::{ctx_menu, history, outline, quick_open, search};
use std::path::Path;
use std::time::{Duration, Instant};
use veil_core::{task_span, BlockContainer, BlockId, BlockKind, Range};
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

/// mermaid.live 打开结果的用户可见反馈；源码已在调用前复制，失败时给出手动恢复路径。
pub(crate) fn mermaid_live_open_notice(result: Result<(), String>) -> Option<String> {
    result
        .err()
        .map(|e| tf(I18nKey::NoticeMermaidLiveOpenFailed, &[&clip_error(&e)]))
}

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
        // 单脏块原位编辑走增量索引（只换该块 + 平移后缀区间），
        // 几何在下面 ensure 之后统一重算一次；其余编辑回退全量重建。
        self.refresh_index_for_edit(&dirty);
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
        // 布局/表格已就绪：高度来源稳定后才累计 top；单块原位编辑只更新邻域。
        self.recompute_geometry_after_edit();
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
        self.route_key_with_focus(key);
        crate::perf::key(started.elapsed());
    }

    /// 单个内部键的路由与执行：`on_key` 与测试共用同一入口。
    /// 模式优先级与各模式的按键语义只在 `mode::route` 一处表达。
    pub(crate) fn route_key(&mut self, key: Key) {
        // 图片放大预览优先：打开时它是唯一输入接收者（缩放/平移/Esc 关闭）。
        if self.ui_state.image_preview.is_some() {
            self.image_preview_key(&key);
            return;
        }
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
            Cmd::SearchToggleRegex => self.search_toggle_regex(),
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
        if self.ui_state.modal.is_some()
            || self.ui_state.settings.is_some()
            || self.ui_state.image_preview.is_some()
            || matches!(self.mode(), Mode::Menu | Mode::History)
        {
            // 模态/设置/图片预览独占输入；菜单/只读预览不挂组合串（避免切走后残留下划线）。
            self.editor.clear_preedit();
            return;
        }
        self.editor.set_preedit(text, cursor);
        self.request_redraw();
    }

    /// IME 提交：按落点分流——模态转键盘助记符，菜单吞掉，查找栏进查询串，否则进文档。
    pub(crate) fn ime_commit(&mut self, text: String) {
        // 图片预览不接收文本：吞掉上屏，避免写到预览背后的正文。
        if self.ui_state.image_preview.is_some() {
            self.editor.clear_preedit();
            return;
        }
        // Windows 下普通字母也可能经 IME Commit 到达（例如中文输入法处于英文模式）。
        // 模态/设置独占输入：必须复用它们自己的键盘路由，不能绕过直接写正文。
        if self.ui_state.modal.is_some() || self.ui_state.settings.is_some() {
            self.editor.clear_preedit();
            if !text.is_empty() {
                self.route_key(Key::Text(text));
            }
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
                    // 菜单/版本历史：吞掉输入，预览保持只读；组合态一并收掉。
                    self.editor.clear_preedit();
                    return;
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
            Action::CopyCodeBlock => self.copy_code_block_at_caret(),
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
        self.window_state.scroll_y = self.window_state.goal_y;
        self.set_ime_area();
        self.ui_state.chrome.notice = tf(
            I18nKey::NoticeZoom,
            &[&((zoom * 100.0).round() as u32).to_string()],
        );
        crate::app::record_diag("zoom", format!("{}%", (zoom * 100.0).round() as u32));
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
            self.ui_state
                .chrome
                .notify_error(t(I18nKey::NoticeImagePasteNeedsFile));
            eprintln!("图片粘贴需要关联文件（草稿无落盘目录）");
            self.request_redraw();
            return;
        };
        match self.ports.assets.save_pasted_image(&doc, img) {
            Ok(rel) => self.insert_paste(&self.image_paragraph_markup(&rel)),
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(I18nKey::NoticeImageSaveFailed, &[&e.to_string()]));
                eprintln!("图片落盘失败（文档未改动）: {e}");
                self.request_redraw();
            }
        }
    }

    /// 将已经渲染的 Mermaid 位图保存为独立 PNG 快照，再插入相对路径。
    /// 插入前按「源码哈希 + 同源码序号」重新定位当前块：0 个匹配或序号越界
    /// （目标块已被删除/改写）都安全丢弃并提示。
    /// 锚点 = 目标块的结束位置（闭围栏之后），`set_caret` 失败同样不插入。
    /// 先落盘、后移动光标和改正文；草稿或落盘失败时原文与光标均不变，也不回退 URL。
    pub(crate) fn insert_mermaid_snapshot(
        &mut self,
        img: &crate::image::ImageData,
        source_hash: u64,
        occurrence: Option<usize>,
    ) {
        let Some(doc) = self.doc.path.clone() else {
            self.ui_state
                .chrome
                .notify_error(t(I18nKey::NoticeMermaidSnapshotNeedsFile));
            self.request_redraw();
            return;
        };
        let Some(end) = self.mermaid_snapshot_anchor(source_hash, occurrence) else {
            self.ui_state
                .chrome
                .notify_error(t(I18nKey::NoticeMermaidSnapshotStale));
            self.request_redraw();
            return;
        };
        match self.ports.assets.save_pasted_image(&doc, img) {
            Ok(rel) => {
                if !self.editor.set_caret(end) {
                    self.ui_state
                        .chrome
                        .notify_error(t(I18nKey::NoticeMermaidSnapshotStale));
                    self.request_redraw();
                    return;
                }
                self.window_state.goal_x = None;
                self.insert_paste(&self.image_paragraph_markup(&rel));
                self.ui_state.chrome.notice = tf(I18nKey::NoticeMermaidImageInserted, &[&rel]);
            }
            Err(e) => {
                self.ui_state.chrome.notify_error(tf(
                    I18nKey::NoticeMermaidSnapshotSaveFailed,
                    &[&e.to_string()],
                ));
                self.request_redraw();
            }
        }
    }

    /// 在当前索引中按源码哈希定位 Mermaid 块，返回闭围栏之后的插入锚点。
    /// `occurrence` 是请求发起时记录的「同源码块序号」：匹配数 >1 时按序号选目标；
    /// 0 个匹配或序号越界（请求的块已被删除/改写）返回 None。
    /// 没有序号（历史调用）时只在唯一匹配下插入，避免猜测用户意图。
    fn mermaid_snapshot_anchor(
        &self,
        source_hash: u64,
        occurrence: Option<usize>,
    ) -> Option<usize> {
        let mut ends = Vec::new();
        for g in &self.index.blocks {
            let is_mermaid = matches!(
                &g.kind,
                BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
            ) || g.kind == BlockKind::Mermaid;
            if !is_mermaid {
                continue;
            }
            let hash = self
                .editor
                .session()
                .fenced_code_source(g.id)
                .map(|(_, body)| crate::app::content_hash(body.as_bytes()));
            if hash == Some(source_hash) {
                ends.push(g.range.end);
            }
        }
        match occurrence {
            Some(i) => ends.get(i).copied(),
            None if ends.len() == 1 => ends.first().copied(),
            None => None,
        }
    }

    /// 记录光标所在 Mermaid 块在同源码块中的序号（0-based，按文档顺序）。
    pub(crate) fn mermaid_occurrence_at(&self, off: usize) -> Option<usize> {
        let target = self.index.blocks.iter().find(|g| {
            g.range.start <= off
                && off <= g.range.end
                && (matches!(
                    &g.kind,
                    BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
                ) || g.kind == BlockKind::Mermaid)
        })?;
        let target_hash = self
            .editor
            .session()
            .fenced_code_source(target.id)
            .map(|(_, body)| crate::app::content_hash(body.as_bytes()))?;
        let mut n = 0;
        for g in &self.index.blocks {
            let is_mermaid = matches!(
                &g.kind,
                BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
            ) || g.kind == BlockKind::Mermaid;
            if !is_mermaid {
                continue;
            }
            let hash = self
                .editor
                .session()
                .fenced_code_source(g.id)
                .map(|(_, body)| crate::app::content_hash(body.as_bytes()));
            if hash == Some(target_hash) {
                if g.id == target.id {
                    return Some(n);
                }
                n += 1;
            }
        }
        None
    }

    /// 图片文件插入（剪贴板文件列表）：与拖入同一条路径——解码 → 落盘 assets → 独占段插入。
    /// 多个图片文件按顺序各成一段；草稿/落盘失败语义由 `insert_image_data` 统一处理。
    fn insert_image_files(&mut self, paths: &[std::path::PathBuf]) {
        for path in paths {
            match self.ports.images.load(path) {
                Ok(img) => self.insert_image_data(&img),
                Err(e) => {
                    self.ui_state
                        .chrome
                        .notify_error(tf(I18nKey::NoticeImageReadFailed, &[&e.to_string()]));
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
            self.ui_state
                .chrome
                .notify_error(t(I18nKey::NoticeInsertImageNeedsFile));
            self.request_redraw();
            return;
        };
        let Some(img) = self.ports.dialogs.open_image() else {
            return;
        };
        if !crate::image::is_image_path(&img) {
            self.ui_state
                .chrome
                .notify_error(t(I18nKey::NoticeUnsupportedImage));
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
                self.ui_state
                    .chrome
                    .notify_error(tf(I18nKey::NoticeImageReadFailed, &[&e.to_string()]));
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
            self.window_state.scroll_y = self.window_state.goal_y;
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
        self.window_state.scroll_y = self.window_state.goal_y;
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
        // 图片放大预览：点击遮罩关闭；点在图上保持打开（与系统看图器一致），
        // 期间不落光标、不开撤销步、不改文档。
        if self.ui_state.image_preview.is_some() {
            let inside = crate::view::image_preview::image_rect(self)
                .is_some_and(|r| mx >= r.x && mx < r.x + r.w && my >= r.y && my < r.y + r.h);
            if !inside {
                self.close_image_preview();
            }
            return false;
        }
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
                    self.enter_exclusive_overlay();
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
            self.enter_exclusive_overlay();
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
                Some(search::Hit::ToggleRegex) => {
                    self.search_toggle_regex();
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
                    let _ = self.replace_current();
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
                    self.window_state.scroll_y = self.window_state.goal_y;
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
        // 代码块复制按钮优先于落光标：命中即复制并吞掉点击。
        if self.try_copy_code_block() {
            return false;
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
        // 点击已渲染的图片 → 放大预览（Ctrl+点击保持原「落光标显源码」路径）。
        if !self.input_state.mods.ctrl && self.try_open_image_preview() {
            return false;
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
        // 生成目录（`[TOC]`）条目点击：跳转到对应标题。源码态/光标在块内不改写文档。
        if self.try_jump_toc_on_click() {
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
        let (tip, tip_is_link) = if !inside
            || self.ui_state.modal.is_some()
            || self.window_state.dragging
            || self.ui_state.tables.drag.is_some()
        {
            (None, false)
        } else {
            match self.hit_offset() {
                Some(off)
                    if !self
                        .editor
                        .selection()
                        .is_some_and(|s| s.start <= off && off < s.end) =>
                {
                    // 链接与 abbr 命中共享同一份 inline 解析（原实现各解析一次整块）。
                    match self.inlines_at_offset(off) {
                        Some(nodes) => {
                            if let Some(u) = self.link_at_in(&nodes, off) {
                                (Some(crate::normalize_link_url(&u)), true)
                            } else {
                                (Self::abbr_title_in(&nodes, off).map(str::to_owned), false)
                            }
                        }
                        None => (None, false),
                    }
                }
                _ => (None, false),
            }
        };
        self.ui_state.hover_link_is_link = tip.is_some() && tip_is_link;
        self.apply_pointer();
        self.set_hover_link(tip, x, y);
    }

    /// 指针形状映射：模态/按钮/链接/大纲/快速打开 = 手型；正文 = 文本；其余 = 默认。
    /// 仅在形状变化时写窗口，避免每次鼠标移动都下发。
    pub(crate) fn apply_pointer(&mut self) {
        use winit::window::CursorIcon;
        let icon = if self.ui_state.modal.is_some() || self.ui_state.image_preview.is_some() {
            CursorIcon::Default
        } else if self.ui_state.search.hover.is_some()
            || self.ui_state.quick.hover.is_some()
            || self.ui_state.outline.hover.is_some()
            || self.code_copy_hit().is_some()
            || self.ui_state.hover_link_is_link
        {
            CursorIcon::Pointer
        } else {
            let vp = self.viewport();
            let (mx, my) = (
                self.input_state.mouse.x as f32,
                self.input_state.mouse.y as f32,
            );
            if mx >= vp.x && mx < vp.x + vp.w && my >= vp.y && my < vp.y + vp.h {
                CursorIcon::Text
            } else {
                CursorIcon::Default
            }
        };
        if self.window_state.pointer == Some(icon) {
            return;
        }
        self.window_state.pointer = Some(icon);
        if let Some(w) = &self.window_state.window {
            w.set_cursor(icon);
        }
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
        self.sync_ime_for_modal();
        self.request_redraw();
    }

    /// 设置模态结论落地：
    /// - OK（BTN_OK）：把表单值应用到 Config，写 config.toml（ConfigSave::All），刷新 UI（主题/字体/滚动等立即生效）。
    /// - Reset Defaults（BTN_RESET_DEFAULTS）：只在表单内恢复默认（用户还可以 OK/Cancel）。
    /// - Cancel（BTN_CANCEL）：关闭不写盘。
    pub(crate) fn resolve_settings(&mut self, id: u16) {
        use crate::view::settings::{BTN_CANCEL, BTN_OK, BTN_RESET_DEFAULTS};
        let Some(s) = self.ui_state.settings.as_mut() else {
            return;
        };
        match id {
            BTN_OK => {
                // 应用表单值到 Config
                let previous_window = self.config.window;
                self.config = s.form.apply(&self.config);
                // 清除/恢复默认几何（DEF-T07-002）：本次 apply 明确移除了窗口几何时，
                // 必须一并丢弃本会话的几何脏位，否则关闭时 finish_close 会把此前
                // resize 得到的当前几何写回，覆盖刚清除的结果。用户之后再次移动/缩放
                // 窗口会重新置脏，届时恢复正常持久化。
                if self.config.window != previous_window {
                    self.window_state.geom_dirty = false;
                    if self.config.window.is_none() {
                        self.window_state.geom = None;
                    }
                }
                // 持久化完整 Config；失败时 save_config 已写入可见错误，不得被
                // 后续成功提示覆盖（DEF-T10-008）。
                let saved = self.save_config(ConfigSave::All);
                // 立即刷新所有受影响的 UI（主题模式、字体等）
                self.refresh_settings_effects();
                if saved {
                    self.ui_state.chrome.notice = t(I18nKey::NoticeSettingsSaved).into();
                }
                self.ui_state.settings = None;
                self.sync_ime_for_modal();
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
                self.sync_ime_for_modal();
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// 设置应用后立即生效：主题模式、字体缓存。不要求重启。
    ///
    /// 走 `refresh_theme_visuals` 而非只换 `Theme` 值：主题代次必须递增，否则
    /// Dark→System（系统为浅色）时布局/光栅缓存仍按旧深色主题命中，正文保持浅灰。
    pub(crate) fn refresh_settings_effects(&mut self) {
        // 1) 主题：mode 变了需要重新取系统深色并刷新 Theme 派生值
        if self.config.mode == ThemeMode::System {
            if let Some(dark) = self.ports.system.is_dark() {
                self.input_state.system_dark = dark;
            }
        }
        self.input_state.theme = Theme::of(self.config.mode, self.input_state.system_dark);
        // 2) 正文排版：字号/行高/字体族先落到排版引擎；它们同时进入 `BaseKey`，
        //    随后的主题代次递增会按新排版输入整体重排（无需重启）。
        self.engine.set_doc_typography(
            self.config.doc_font_size,
            self.config.doc_line_height,
            self.config.doc_font_family,
        );
        // 4) 文档字体族/字号/行高/制表符 + 主题配色：统一由主题刷新入口
        //    递增代次、重排可见块并重算几何（base_key 同时含 typography/theme）。
        self.refresh_theme_visuals();
        self.set_ime_area();
        // 成功/失败提示由 resolve_settings 按落盘结果决定，这里不写 notice。
    }

    /// 切换专注模式（阶段 1）：只改视觉开关，不动真值/派生缓存；重绘即生效。
    /// 与打字机模式互斥：开启专注时关闭打字机。
    pub(crate) fn toggle_focus(&mut self) {
        let on = !self.ui_state.writing.focus;
        self.ui_state.writing.focus = on;
        if on {
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

    /// 切换打字机模式：开启时关闭专注模式；滚动锚点只在 `scroll_to_caret` 生效。
    pub(crate) fn toggle_typewriter(&mut self) {
        let on = !self.ui_state.writing.typewriter;
        self.ui_state.writing.typewriter = on;
        if on {
            self.ui_state.writing.focus = false;
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
        if let Some(top) = top {
            let vp = self.viewport();
            let max = chrome::max_scroll(vp, self.doc_height());
            let target = (top - MARGIN * self.input_state.scale).clamp(0.0, max);
            self.window_state.goal_y = target;
            self.window_state.scroll_y = self.window_state.goal_y;
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
        // 瞬时滚动：写入目标后立即同步，无需动画插值。
        self.window_state.goal_y = (self.window_state.goal_y - d).clamp(0.0, max);
        self.window_state.scroll_y = self.window_state.goal_y;
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

    /// 「正则模式」开关（点击 `. *` 或 Alt+R）：翻转后立即重扫，会话内保持。
    /// 关闭时查询串回到逐字面量匹配，不改变已输入内容。
    pub(crate) fn search_toggle_regex(&mut self) {
        if !self.ui_state.search.open {
            return;
        }
        self.ui_state.search.regex = !self.ui_state.search.regex;
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
        // 替换串落盘后长度可能变化：以「旧起点 + 实际写入字节数」为下一处起点。
        // 正则模式下写入长度由捕获组展开决定，不能用模板原串长度。
        let Some(inserted) = self.replace_current() else {
            return;
        };
        let from = m.start + inserted;
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
    /// 返回实际写入的字节数（正则捕获展开后可能不同于模板长度），供调用方定位下一处。
    pub(crate) fn replace_current(&mut self) -> Option<usize> {
        if !self.ui_state.search.open || !self.ui_state.search.replace_open {
            return None;
        }
        let m = self.ui_state.search.current_match()?;
        let text = self.replacement_text(m);
        let dirty = self.editor.replace_range(m, &text)?;
        self.window_state.goal_x = None;
        self.after_edit(dirty);
        self.search_rescan(); // 命中缓存重建：后续替换绝不基于旧区间
        Some(text.len())
    }

    /// 单处替换文本：正则模式下按捕获组展开模板（`$1`/`${name}`），字面量模式原样使用。
    /// 展开失败（理论上不会，命中来自同一模式）时回退为模板原文，保证替换可用。
    fn replacement_text(&self, m: veil_core::Range) -> String {
        let st = &self.ui_state.search;
        if !st.regex {
            return st.replace.clone();
        }
        let n = self.editor.session().text_len();
        let hay = self
            .editor
            .session()
            .text_slice(veil_core::Range::new(0, n))
            .unwrap_or_default();
        search::expand_one(&hay, &st.query, st.case_sensitive, m, &st.replace)
            .unwrap_or_else(|| st.replace.clone())
    }

    /// 全部替换：一次事务覆盖所有命中（单撤销步），随后重建命中缓存。
    pub(crate) fn replace_all(&mut self) {
        if !self.ui_state.search.open || !self.ui_state.search.replace_open {
            return;
        }
        let st = &self.ui_state.search;
        let outcome = if st.regex {
            // 正则：逐命中展开捕获组，每处替换文本可能不同。
            let n = self.editor.session().text_len();
            let hay = self
                .editor
                .session()
                .text_slice(veil_core::Range::new(0, n))
                .unwrap_or_default();
            let Ok(edits) =
                search::expand_replacements(&hay, &st.query, st.case_sensitive, &st.replace)
            else {
                return;
            };
            self.editor.replace_all_with(&edits)
        } else {
            let matches = st.matches.clone();
            let text = st.replace.clone();
            self.editor.replace_all(&matches, &text)
        };
        let Some((dirty, _)) = outcome else {
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
pub(crate) fn mermaid_body_is_empty(body: &str) -> bool {
    !body
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !l.starts_with("%%"))
}

/// 解析脚注定义块首行 `[^id]: ` 的 id。
/// 为什么在这里重写一遍：`layout::footnote_def_prefix` 是排版私有实现，
/// 交互层只借它定位命中，不应把排版模块的内部约定变成公共 API。
/// 规则与 core `parse_footnote_def` 同口径（缩进 ≤3、字母数字/-/_、`]:`）。
pub(crate) fn footnote_def_id(text: &str) -> Option<&str> {
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

    /// Front Matter 是否按「非活动态隐藏」处理（REN-SWEEP-001）。
    /// 源码模式恒显示原文；光标进入块内（WYSIWYM）显示原文可编辑；
    /// 打印/导出无光标语义，与 HTML 导出保持一致直接隐藏。
    pub(crate) fn frontmatter_hidden(&self, g: &BlockGeom, caret: usize) -> bool {
        self.block_is_frontmatter(g)
            && !self.source_mode()
            && (self.window_state.print.is_some() || !crate::view::node_active(g.range, caret))
    }

    /// 文档顶部 YAML/TOML Front Matter 解析为 `FrontMatter` 块；但夹具中
    /// 也常见「正文中间单独成段的 TOML Front Matter」示例——内核只认文档首行，
    /// 视图层按同构语法识别段落形态的 TOML Front Matter（不改变块结构/字节），
    /// 使其在 WYSIWYG 非活动态同样隐藏（REN-SWEEP-001）。
    /// 判定结果由索引期缓存（`BlockGeom::frontmatter_like`），此处只读标志：
    /// 每帧 O(块数) 切片解析会拖慢大文档真实击键（DEF-RENDER-FINAL-001）。
    pub(crate) fn block_is_frontmatter(&self, g: &BlockGeom) -> bool {
        g.frontmatter_like
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
        self.block_height_with(g, self.base_key(), self.effective_caret())
    }

    /// `block_height` 的内核：基础键与光标由调用方预先取好，供几何重算的
    /// O(块数) 循环复用，避免每块重复构造全文派生键。
    pub(crate) fn block_height_with(
        &self,
        g: &BlockGeom,
        base: crate::view::cache::BaseKey,
        caret: usize,
    ) -> f32 {
        // B3：折叠 details 的隐藏子块不渲染、不占高度（不是光栅裁剪）。
        if self.block_hidden(g) {
            return 0.0;
        }
        // REN-SWEEP-001：非活动态 Front Matter 折叠为零内容高度，但保留一条
        // 可点击折叠条（打印态仍为 0）：否则整块消失后无法用鼠标进入编辑。
        if self.frontmatter_hidden(g, caret) {
            if self.window_state.print.is_some() {
                return 0.0;
            }
            return (crate::tokens::doc::FM_COLLAPSED_H * self.input_state.scale).round();
        }
        // Details 头块折叠态：只显示 summary 单行 + ▼/▶ 箭头。折叠态是纯 UI 状态，
        // 不参与 layout 缓存 key；直接在 block_height 层面钳高度，避免缓存污染。
        // 光标进入 summary 或右键菜单落光标时也保持同一折叠几何。
        // 打印态恒展开（用户想看完整内容），跳过折叠钳制。
        if let BlockKind::Details { .. } = &g.kind {
            if self.window_state.print.is_none() && !self.ui_state.expanded_details.contains(&g.id)
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
                base,
                caret,
                self.toc_layout_epoch(g),
            )
            .map_or(g.est_h, |l| l.height)
    }

    /// 每视觉行可容纳的字符数（按平均字宽 0.5em 估算，仅用于便宜高度）。
    pub(crate) fn chars_per_line(&self) -> f32 {
        (self.content_width() / (0.5 * BASE_FONT_SIZE * self.input_state.scale)).max(1.0)
    }

    /// 光标所在块索引；文末光标可能落在最后一块的尾换行之后，回退末块。
    pub(crate) fn caret_block_index(&self) -> Option<usize> {
        self.block_index_at_offset(self.editor.caret())
    }

    /// 字节偏移所在块的下标（块区间按文档序排列，二分定位 O(log 块数)）。
    pub(crate) fn block_index_at_offset(&self, offset: usize) -> Option<usize> {
        let blocks = &self.index.blocks;
        let i = blocks.partition_point(|g| g.range.end < offset);
        if blocks
            .get(i)
            .is_some_and(|g| g.range.start <= offset && offset <= g.range.end)
        {
            return Some(i);
        }
        if offset >= self.editor.session().text_len() {
            return blocks.len().checked_sub(1);
        }
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_live_open_success_has_no_error_notice() {
        assert_eq!(mermaid_live_open_notice(Ok(())), None);
    }

    #[test]
    fn mermaid_live_open_failure_explains_manual_recovery() {
        let notice = mermaid_live_open_notice(Err("浏览器不可用\n详细堆栈".into()))
            .expect("打开失败必须产生用户可见提示");
        assert!(notice.contains("无法打开 mermaid.live：浏览器不可用"));
        assert!(notice.contains("Mermaid 源码已复制"));
        assert!(notice.contains("手动打开 https://mermaid.live/ 并粘贴源码"));
        assert!(!notice.contains("详细堆栈"), "状态栏只显示首行错误");
    }
}
