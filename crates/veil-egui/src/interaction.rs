//! 交互策略：按键到意图的翻译、编辑后的几何刷新编排、光标移动/命中/点选/
//! 拖选、剪贴板、菜单动作。只发意图与调几何，不做像素绘制。

use crate::app::App;
use crate::chrome::{self, MenuAction};
use crate::document::block_index::BlockGeom;
use crate::editor::Intent;
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT, MARGIN};
use crate::html::html_to_markdown;
use crate::input::{Action, Key};
use crate::mode::{route, Cmd, Mode, TextSink};
use crate::theme::ThemeMode;
use crate::view::{outline, quick_open, search};
use std::time::{Duration, Instant};
use veil_core::{task_span, BlockId, BlockKind};
use winit::event::ElementState;
use winit::keyboard::{Key as WKey, NamedKey};

/// 连续点击（双击/三击）的最大间隔。
const CLICK_GAP: Duration = Duration::from_millis(500);

/// 连续点击允许的位置漂移（逻辑像素）。
const CLICK_SLOP: f64 = 4.0;

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
        self.request_redraw();
    }

    pub(crate) fn on_key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        self.route_key(key);
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
                self.request_redraw();
            }
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
            Cmd::SearchToggleField => self.search_toggle_field(),
            Cmd::SearchNext => self.search_next(),
            Cmd::SearchPrev => self.search_prev(),
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
    pub(crate) fn perform_action(&mut self, action: Action) {
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
            Action::CloseMenu => self.ui_state.chrome.close(),
            Action::ToggleOutline => self.toggle_outline(),
            Action::Export => self.export_now(),
            Action::Find => self.open_search(),
            Action::Replace => self.open_replace(),
        }
    }

    /// 粘贴（切片 11/14）：HTML 优先转 Markdown，转换无内容（纯图片/空结构）回退纯文本；
    /// 文本皆空时尝试位图：落盘 `<doc>.assets/` 并插入 `![](相对路径)`。
    /// 草稿（无关联文件）拒绝图片粘贴（无写入基准），文档不变、不置脏。
    pub(crate) fn paste_clipboard(&mut self) {
        let md = self
            .ports
            .clipboard
            .get_html()
            .map(|h| html_to_markdown(&h))
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
            Ok(rel) => self.insert_paste(&format!("![]({rel})")),
            Err(e) => {
                self.ui_state.chrome.notice = format!("图片落盘失败: {e}");
                eprintln!("图片落盘失败（文档未改动）: {e}");
                self.request_redraw();
            }
        }
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

    /// 执行菜单动作。返回 true = 退出应用。
    pub(crate) fn perform_menu(&mut self, action: MenuAction) -> bool {
        match action {
            MenuAction::New => self.new_draft(),
            MenuAction::Open => self.open_via_dialog(),
            MenuAction::Save => self.save_now(),
            MenuAction::SaveAs => self.save_as(),
            MenuAction::Export => self.export_now(),
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
            MenuAction::ToggleOutline => self.toggle_outline(),
            MenuAction::Find => self.open_search(),
            MenuAction::Replace => self.open_replace(),
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

    /// 左键按下路由（菜单 > 菜单栏 > 滚动条 > 状态栏 > 正文光标）。
    /// 返回 true = 退出应用。菜单打开时点击外部只关菜单、不移动光标。
    pub(crate) fn on_left_click(&mut self) -> bool {
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        let scale = self.input_state.scale;
        let vp = self.viewport();
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

    /// 悬停高亮（菜单标题 / 弹出菜单项 / 大纲条目）。
    pub(crate) fn update_hover(&mut self) {
        let (x, y) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
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

    /// 切换专注模式（阶段 1）：只改视觉开关，不动真值/派生缓存；重绘即生效。
    pub(crate) fn toggle_focus(&mut self) {
        let on = !self.ui_state.writing.focus;
        self.ui_state.writing.focus = on;
        self.ui_state.chrome.notice = format!("专注模式：{}", if on { "开" } else { "关" });
        self.request_redraw();
    }

    /// 切换打字机模式（阶段 1）：开启即刻把光标行移到视口锚点；之后每次
    /// `scroll_to_caret`（编辑/垂直移动）都保持该锚点。
    pub(crate) fn toggle_typewriter(&mut self) {
        let on = !self.ui_state.writing.typewriter;
        self.ui_state.writing.typewriter = on;
        if on {
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

    /// 滚轮：快速打开浮层内滚动候选，指针在大纲面板内时滚动面板，否则滚动正文。
    pub(crate) fn on_wheel(&mut self, d: f32) {
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

    /// 打开浮层（Ctrl+P / 菜单）：关闭菜单与查找栏，重建候选（最近文件 + 当前目录）。
    pub(crate) fn open_quick(&mut self) {
        self.ui_state.chrome.close();
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
        self.editor.clear_preedit();
        self.ui_state.search.open = true;
        self.ui_state.search.replace_open = false;
        self.ui_state.search.clear();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 打开查找替换栏（Ctrl+H / 菜单）：未打开时先开栏；查询保留，焦点给替换串。
    pub(crate) fn open_replace(&mut self) {
        self.close_quick();
        self.ui_state.chrome.close();
        self.ui_state.search.open = true;
        self.ui_state.search.replace_open = true;
        self.ui_state.search.focus = search::Field::Replace;
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

/// winit 键 → 内部 `Key`。ctrl 按住时 text 为空，字母键退化为 Z/Y 供快捷键判定。
pub(crate) fn map_key(event: &winit::event::KeyEvent) -> Option<Key> {
    if let Some(t) = &event.text {
        if !t.is_empty() && !t.chars().any(|c| c.is_control()) {
            return Some(Key::Text(t.to_string()));
        }
    }
    match &event.logical_key {
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
        _ => None,
    }
}

// ── 几何编排（原 geometry.rs）：视口区间/块高/光标坐标/Reveal 指纹 ────────────

impl App {
    pub(crate) fn content_width(&self) -> f32 {
        let (w, _) = self.win_size();
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
                .table(g.id, self.base_key())
                .map_or(g.est_h, |t| t.height);
        }
        self.cache
            .layout(g.id, &g.kind, g.range, self.base_key(), self.editor.caret())
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

    /// 光标屏幕矩形（物理像素，含菜单栏偏移）。(x, y, h)；找不到所在块（空文档）时落视口原点。
    pub(crate) fn caret_screen(&mut self) -> (f32, f32, f32) {
        let caret = self.editor.caret();
        let vp = self.viewport();
        if let Some(i) = self.caret_block_index() {
            let (id, start, top) = {
                let g = &self.index.blocks[i];
                (g.id, g.range.start, g.top)
            };
            // 视口外块可能尚未布局：光标路径按需补齐
            self.ensure_block(id);
            if let Some(l) = self.cache.layouts.get(&id) {
                let (x, y, h) = self.engine.caret_xy(l, caret - start);
                return (
                    MARGIN * self.input_state.scale + x,
                    vp.y + top + y - self.window_state.scroll_y,
                    h,
                );
            }
        }
        (
            MARGIN * self.input_state.scale,
            vp.y + MARGIN * self.input_state.scale - self.window_state.scroll_y,
            BASE_LINE_HEIGHT * self.input_state.scale,
        )
    }
}
