//! 交互策略：按键到意图的翻译、编辑后的几何刷新编排、光标移动/命中/点选/
//! 拖选、剪贴板、菜单动作。只发意图与调几何，不做像素绘制。

use crate::app::App;
use crate::chrome::{self, MenuAction};
use crate::document::block_index::BlockGeom;
use crate::editor::Intent;
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT, MARGIN, PARA_GAP};
use crate::html::html_to_markdown;
use crate::input::{Action, Key, Mods};
use crate::mode::{route, Cmd, Mode, TextSink};
use crate::paint::{
    active_cell_index, table_cell_start, table_delete_column, table_delete_row, table_insert_column,
    table_insert_row, table_set_align,
};
use crate::theme::ThemeMode;
use crate::view::{ctx_menu, outline, quick_open, search};
use crate::view::ctx_menu::{CtxAction, CtxKind, TableCtx};
use std::path::Path;
use std::time::{Duration, Instant};
use veil_core::{task_span, BlockId, BlockKind, CellAlign, InlineKind, InlineNode, Range};
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
        let cmd = route(
            self.mode(),
            key,
            self.input_state.mods,
            self.editor.preedit().is_some(),
        );
        match cmd {
            Cmd::Ignore => {}
            Cmd::OpenQuick => self.open_quick(),
            Cmd::CloseMenu => {
                self.ui_state.chrome.close();
                self.ui_state.ctx.close();
                self.request_redraw();
            }
            Cmd::MenuMove(d) => self.menu_move(d),
            Cmd::MenuAccept => self.menu_accept(),
            // 快速打开浮层动作自带重绘与 IME 区域刷新
            Cmd::QuickClose => self.close_quick(),
            Cmd::QuickAccept => self.quick_accept(),
            Cmd::QuickMove(d) => self.quick_step(d),
            Cmd::QuickBackspace => self.quick_backspace(),
            Cmd::QuickInput(t) => self.quick_input(&t),
            // 查找栏动作自带重绘与 IME 区域刷新
            Cmd::CloseSearch => self.close_search(),
            Cmd::SearchInput(t) => self.search_input(&t),
            Cmd::SearchBackspace => self.search_backspace(),
            Cmd::SearchPaste => self.search_paste(),
            Cmd::SearchToggleField => self.search_toggle_field(),
            Cmd::SearchNext => self.search_next(),
            Cmd::SearchPrev => self.search_prev(),
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
        self.editor.set_preedit(text, cursor);
        self.request_redraw();
    }

    /// IME 提交：按落点分流——菜单打开时吞掉，查找栏进查询串，否则进文档。
    pub(crate) fn ime_commit(&mut self, text: String) {
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
                if self.mode() == Mode::Menu {
                    return; // 菜单打开：吞掉输入
                }
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
                if let Some(dirty) = self.editor.apply(intent) {
                    self.after_edit(dirty);
                }
            }
            // 垂直移动/扩选：保留 goal_x（连续上下不漂移）
            Action::MoveVertical(delta) => self.move_vertical(delta, false),
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
            Action::QuickOpen => self.open_quick(),
            Action::CloseMenu => {
                self.ui_state.chrome.close();
                self.ui_state.ctx.close();
            }
            Action::ToggleOutline => self.toggle_outline(),
            Action::ToggleSourceMode => self.toggle_source_mode(),
            Action::Export => self.export_now(),
            Action::ExportPdf => self.export_pdf(),
            Action::Print => self.print_document(),
            Action::Find => self.open_search(),
            Action::Replace => self.open_replace(),
            Action::InsertImage => self.insert_image_via_dialog(),
            Action::ZoomIn => self.zoom_step(1),
            Action::ZoomOut => self.zoom_step(-1),
            Action::ZoomReset => self.zoom_reset(),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
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
            "全屏（F11 退出）"
        } else {
            "已退出全屏"
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
        self.window_state.scroll_y = self.window_state.scroll_y.clamp(0.0, max);
        self.set_ime_area();
        self.ui_state.chrome.notice = format!("缩放 {}%", (zoom * 100.0).round() as u32);
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
            self.ui_state.chrome.notice = "图片粘贴需要关联文件（草稿无落盘目录）".into();
            eprintln!("图片粘贴需要关联文件（草稿无落盘目录）");
            self.request_redraw();
            return;
        };
        match self.ports.assets.save_pasted_image(&doc, img) {
            Ok(rel) => self.insert_paste(&self.image_paragraph_markup(&rel)),
            Err(e) => {
                self.ui_state.chrome.notice = format!("图片落盘失败: {e}");
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
                    self.ui_state.chrome.notice = format!("图片读取失败: {e}");
                    eprintln!("图片读取失败（文档未改动）{}: {e}", path.display());
                    self.request_redraw();
                }
            }
        }
    }

    /// 插入图片入口（2f）：菜单「插入图片…」→ 系统对话框选图 → 插入相对路径链接。
    /// 草稿（无关联文件）拒绝（无相对路径基准）；非图片扩展名拒绝；取消无副作用。
    pub(crate) fn insert_image_via_dialog(&mut self) {
        let Some(doc) = self.doc.path.clone() else {
            self.ui_state.chrome.notice = "插入图片需要关联文件（草稿无相对路径基准）".into();
            self.request_redraw();
            return;
        };
        let Some(img) = self.ports.dialogs.open_image() else {
            return;
        };
        if !crate::image::is_image_path(&img) {
            self.ui_state.chrome.notice = "所选文件不是支持的图片格式".into();
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
        // 拖放是文档级动作：先收起浮层/菜单，避免编辑与浮层共存
        self.close_quick();
        self.ui_state.chrome.close();
        if !crate::image::is_image_path(&path) {
            self.open_path(path);
            return;
        }
        self.point_caret(false); // 图片插入点 = 鼠标落点
        match self.ports.images.load(&path) {
            Ok(img) => self.insert_image_data(&img),
            Err(e) => {
                self.ui_state.chrome.notice = format!("图片读取失败: {e}");
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
        let Some(bi) = self
            .index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
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
            let next_i = if delta < 0 {
                bi.checked_sub(1)
            } else if bi + 1 < self.index.blocks.len() {
                Some(bi + 1)
            } else {
                None
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
        let x = (self.input_state.mouse.x as f32 - MARGIN * self.input_state.scale).max(0.0);
        let y = (self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y).max(0.0);
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let g = blocks
                .iter()
                .rev()
                .find(|g| g.top <= y)
                .or_else(|| blocks.first())?;
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
        let x = self.input_state.mouse.x as f32 - MARGIN * self.input_state.scale;
        let y = self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y;
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let Some(g) = blocks
                .iter()
                .rev()
                .find(|g| g.top <= y)
                .or_else(|| blocks.first())
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
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect()
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
            let n = self.ui_state.ctx.items().len();
            if n == 0 {
                return;
            }
            let last = n as i32 - 1;
            let i = match self.ui_state.ctx.hover {
                Some(i) => (i as i32 + d).clamp(0, last) as usize,
                None if d < 0 => n - 1,
                None => 0,
            };
            self.ui_state.ctx.hover = Some(i);
            self.request_redraw();
            return;
        }
        let Some(open) = self.ui_state.chrome.open else {
            return;
        };
        let recent = self.recent_labels();
        let n = chrome::menu_rows(open, &recent).len();
        if n == 0 {
            return;
        }
        let last = n as i32 - 1;
        // 只认当前菜单的选中项：切换菜单后残留的旧下标不得参与移动
        let current = match self.ui_state.chrome.hover_item {
            Some((m, i)) if m == open && i < n => Some(i),
            _ => None,
        };
        let i = match current {
            Some(i) => (i as i32 + d).clamp(0, last) as usize,
            None if d < 0 => n - 1,
            None => 0,
        };
        self.ui_state.chrome.hover_item = Some((open, i));
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
        self.ui_state.chrome.close();
        self.request_redraw();
        if self.perform_menu(action) {
            self.window_state.exit_requested = true;
        }
    }

    /// 执行菜单动作。返回 true = 退出应用。
    pub(crate) fn perform_menu(&mut self, action: MenuAction) -> bool {
        match action {
            MenuAction::New => self.new_draft(),
            MenuAction::Open => self.open_via_dialog(),
            MenuAction::Save => self.save_now(),
            MenuAction::SaveAs => self.save_as(),
            MenuAction::Export => self.export_now(),
            MenuAction::ExportPdf => self.export_pdf(),
            MenuAction::Print => self.print_document(),
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
            MenuAction::ZoomIn => self.zoom_step(1),
            MenuAction::ZoomOut => self.zoom_step(-1),
            MenuAction::ZoomReset => self.zoom_reset(),
            MenuAction::ToggleFullscreen => self.toggle_fullscreen(),
            MenuAction::ToggleOutline => self.toggle_outline(),
            MenuAction::Find => self.open_search(),
            MenuAction::Replace => self.open_replace(),
            MenuAction::InsertImage => self.insert_image_via_dialog(),
            MenuAction::ToggleBold => self.perform_action(Action::Doc(Intent::ToggleWrap("**"))),
            MenuAction::ToggleItalic => self.perform_action(Action::Doc(Intent::ToggleWrap("*"))),
            MenuAction::InsertLink => self.perform_action(Action::Doc(Intent::InsertLink)),
            MenuAction::OpenRecent(i) => self.open_recent(i),
            MenuAction::QuickOpen => self.open_quick(),
            MenuAction::Quit => return true,
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
        if self.ui_state.quick.open || self.ui_state.search.open {
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
        let (kind, link) = if in_sel {
            (CtxKind::Selection, None)
        } else {
            self.editor.set_caret(off);
            self.window_state.goal_x = None;
            match self.table_ctx_at(off) {
                Some(tc) => (CtxKind::Table(tc), None),
                None => match self.link_at(off) {
                    Some(url) => (CtxKind::Link, Some(url)),
                    None => (CtxKind::Block, None),
                },
            }
        };
        self.ui_state.ctx.open_at(mx, my, kind, link);
        self.set_ime_area();
        self.request_redraw();
    }

    /// 偏移所在表格的行列上下文：命中单元格直接取，竖线/分隔行取同一行最近单元格。
    /// 表头（及分隔行）`body_row` 为 None，只给列操作。
    fn table_ctx_at(&self, off: usize) -> Option<TableCtx> {
        let g = self.index.blocks.iter().find(|g| {
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

    /// 命中偏移处的 inline 链接 URL（含标记区命中；引用式链接无 url 区间不识别）。
    fn link_at(&self, offset: usize) -> Option<String> {
        let g = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= offset && offset < g.range.end)?;
        let nodes = self.editor.session().block_inlines(g.id)?;
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

    /// 执行右键菜单动作（菜单随后由调用方关闭；CopyLink 载荷读自 ctx.link）。
    fn ctx_exec(&mut self, action: CtxAction) {
        match action {
            CtxAction::Cut => self.cut_selection(),
            CtxAction::Copy => self.copy_selection(),
            CtxAction::Paste => self.paste_clipboard(),
            CtxAction::SelectAll => self.perform_action(Action::Doc(Intent::SelectAll)),
            CtxAction::CopyLink => {
                if let Some(u) = self.ui_state.ctx.link.clone() {
                    self.ports.clipboard.set(&u);
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
        }
        self.request_redraw();
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
            .find(|g| {
                g.kind == BlockKind::Table && g.range.start <= caret && caret <= g.range.end
            })
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
            CtxAction::TableDeleteColumn => (
                tc.body_row.map_or(0, |r| 2 + r),
                tc.col.min(tc.ncols - 2),
            ),
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

    /// 左键按下路由（右键菜单 > 菜单 > 菜单栏 > 滚动条 > 状态栏 > 正文光标）。
    /// 返回 true = 退出应用。菜单打开时点击外部只关菜单、不移动光标。
    pub(crate) fn on_left_click(&mut self) -> bool {
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        let scale = self.input_state.scale;
        let vp = self.viewport();
        // 右键菜单打开中：命中项执行；其余位置（含面板内空白）关闭并吞掉
        if self.ui_state.ctx.open {
            let l = self.ctx_layout();
            let hit = ctx_menu::hit(&l, self.ui_state.ctx.items().len(), mx, my);
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
            if let Some(item) = chrome::hit_popup_item(mx, my, open, scale, &recent) {
                self.ui_state.chrome.close();
                if let Some(action) = chrome::action_at(open, item, &recent) {
                    return self.perform_menu(action);
                }
                return false;
            }
            if chrome::popup_contains(mx, my, open, scale, &recent) {
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
            match search::hit(&l, scale, self.ui_state.search.replace_open, mx, my) {
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
                    self.window_state.scroll_y =
                        chrome::page_scroll(vp, doc_h, self.window_state.scroll_y, dir);
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
            let h = ctx_menu::hit(&l, self.ui_state.ctx.items().len(), x, y);
            if self.ui_state.ctx.hover != h {
                self.ui_state.ctx.hover = h;
                self.request_redraw();
            }
            return; // 右键菜单打开时其余悬停不更新
        }
        let menu = chrome::hit_menu_bar(x, y, self.input_state.scale);
        let item = self.ui_state.chrome.open.and_then(|open| {
            let recent = self.recent_labels();
            chrome::hit_popup_item(x, y, open, self.input_state.scale, &recent).map(|i| (open, i))
        });
        if self.ui_state.chrome.hover_menu != menu || self.ui_state.chrome.hover_item != item {
            self.ui_state.chrome.hover_menu = menu;
            self.ui_state.chrome.hover_item = item;
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
    }

    /// 切换大纲面板：版心宽变化等价一次 resize，布局/表格键随宽度量化值失配。
    pub(crate) fn toggle_outline(&mut self) {
        self.ui_state.outline.open = !self.ui_state.outline.open;
        self.ui_state.outline.hover = None;
        self.ui_state.outline.scroll = 0.0;
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

    /// 切换专注模式（阶段 1）：只改视觉开关，不动真值/派生缓存；重绘即生效。
    pub(crate) fn toggle_focus(&mut self) {
        let on = !self.ui_state.writing.focus;
        self.ui_state.writing.focus = on;
        if on {
            // 专注 / 打字机互斥（同为「写作模式」，同开语义混乱）
            self.ui_state.writing.typewriter = false;
        }
        self.ui_state.chrome.notice = format!("专注模式：{}", if on { "开" } else { "关" });
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
        self.ui_state.chrome.notice = format!("打字机模式：{}", if on { "开" } else { "关" });
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

    /// 大纲面板矩形（关闭时 None）：右缘贴滚动条左缘，纵向占满内容视口。
    pub(crate) fn outline_rect(&self) -> Option<outline::Rect> {
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
            self.window_state.scroll_y = (top - MARGIN * self.input_state.scale).clamp(0.0, max);
        }
        self.set_ime_area();
        self.request_redraw();
    }

    /// 滚轮：右键菜单打开时吞掉（瞬态菜单不滚动正文）；快速打开浮层内滚动候选，
    /// 指针在大纲面板内时滚动面板，否则滚动正文。
    pub(crate) fn on_wheel(&mut self, d: f32) {
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
        let vp = self.viewport();
        let max = chrome::max_scroll(vp, self.doc_height());
        self.window_state.scroll_y = (self.window_state.scroll_y - d).clamp(0.0, max);
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

    /// 打开浮层（Ctrl+Shift+P / 菜单）：关闭菜单与查找栏，重建候选（最近文件 + 当前目录）。
    pub(crate) fn open_quick(&mut self) {
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
        self.window_state.goal_x = None;
        self.after_edit(Vec::new()); // 补齐可见/光标块布局并滚动入视口
        self.set_ime_area();
    }
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
    // Ctrl+/：部分布局/IME 下逻辑键给 `/` 字符，另一些给物理 Slash 键；两者都认。
    // 仅在 Ctrl 组合时映射，普通 `/` 仍走文本分支，不被吞。
    let ctrl = mods.ctrl && !mods.alt;
    let slash = matches!(&event.logical_key, WKey::Character(c) if c.as_str() == "/")
        || event.physical_key == PhysicalKey::Code(KeyCode::Slash);
    if ctrl && slash {
        return Some(Key::Slash);
    }
    map_key_parts(event.text.as_deref(), event.key_without_modifiers(), mods)
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
    pub(crate) fn content_width(&self) -> f32 {
        let (w, _) = self.win_size();
        // 打印：无滚动条/大纲面板，版心 = 页宽 - 两侧页边距（3b）
        if self.window_state.print.is_some() {
            return (w as f32 - 2.0 * MARGIN * self.input_state.scale).max(32.0);
        }
        let panel = if self.ui_state.outline.open {
            outline::PANEL_W * self.input_state.scale
        } else {
            0.0
        };
        (w as f32
            - 2.0 * MARGIN * self.input_state.scale
            - chrome::SCROLL_W * self.input_state.scale
            - panel)
            .max(32.0)
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
    /// 1 = 同段软换行；≥2 = 段落分隔（markdown 折叠多余空行）。
    fn trailing_newlines(&self, caret: usize) -> usize {
        let mut n = 0;
        let mut c = caret;
        while n < 2 {
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
        let margin = MARGIN * scale;
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
                self.ensure_block(id);
                if let Some(l) = self.cache.layouts.get(&id) {
                    let y = if nl == 1 {
                        top + l.height
                    } else {
                        top + l.height + PARA_GAP * scale
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
            return (margin, vp.y + top + h + PARA_GAP * scale - scroll, line_h);
        }
        (margin, vp.y + margin - scroll, line_h)
    }
}
