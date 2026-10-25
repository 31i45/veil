//! 交互策略：按键到意图的翻译、编辑后的几何刷新编排、光标移动/命中/点选/
//! 拖选、剪贴板、菜单动作。只发意图与调几何，不做像素绘制。

use super::*;
use crate::chrome::MenuAction;
use crate::editor::Intent;
use crate::html::html_to_markdown;
use crate::input::{translate, Action, Key};
use winit::keyboard::{Key as WKey, NamedKey};

impl App {
    /// 编辑后统一入口：失效脏块、视口跟随光标、请求重绘。
    /// 仅内容编辑重启自动保存静默期；纯光标移动不推迟落盘。
    pub(super) fn after_edit(&mut self, dirty: Vec<BlockId>) {
        if !dirty.is_empty() {
            self.last_activity = Some(Instant::now());
        }
        for id in dirty {
            self.layouts.remove(&id);
            self.rasters.remove(&id);
            self.math_imgs.remove(&id);
            self.math_failed.remove(&id);
            self.tables.remove(&id);
        }
        // 先按新块结构重建索引，ensure_* 才能只遍历索引（不再各自全量枚举）
        self.refresh_index();
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

    pub(super) fn on_key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let key = match map_key(event) {
            Some(k) => k,
            None => return,
        };
        // 菜单打开时键盘只响应 Esc（其余按键被菜单吞掉，避免误编辑）
        if self.chrome.is_open() {
            if key == Key::Escape {
                self.chrome.close();
                self.request_redraw();
            }
            return;
        }
        let Some(action) = translate(key, self.mods) else {
            return;
        };
        match action {
            Action::Doc(intent) => {
                self.goal_x = None; // 非垂直移动：重置目标列
                let old_caret = self.editor.caret();
                if let Some(dirty) = self.editor.apply(intent) {
                    self.invalidate_reveal(old_caret, self.editor.caret());
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
            Action::CloseMenu => self.chrome.close(),
            Action::Export => self.export_now(),
        }
        self.set_ime_area();
    }

    /// 粘贴（切片 11/14）：HTML 优先转 Markdown，转换无内容（纯图片/空结构）回退纯文本；
    /// 文本皆空时尝试位图：落盘 `<doc>.assets/` 并插入 `![](相对路径)`。
    /// 草稿（无关联文件）拒绝图片粘贴（无写入基准），文档不变、不置脏。
    pub(super) fn paste_clipboard(&mut self) {
        let md = self
            .clipboard
            .get_html()
            .map(|h| html_to_markdown(&h))
            .filter(|m| !m.trim().is_empty());
        if let Some(text) = md.or_else(|| self.clipboard.get()) {
            self.insert_paste(&text);
            return;
        }
        let Some(img) = self.clipboard.get_image() else {
            return;
        };
        let Some(doc) = self.path.clone() else {
            self.chrome.notice = "图片粘贴需要关联文件（草稿无落盘目录）".into();
            eprintln!("图片粘贴需要关联文件（草稿无落盘目录）");
            self.request_redraw();
            return;
        };
        match self.assets.save_pasted_image(&doc, &img) {
            Ok(rel) => self.insert_paste(&format!("![]({rel})")),
            Err(e) => {
                self.chrome.notice = format!("图片落盘失败: {e}");
                eprintln!("图片落盘失败（文档未改动）: {e}");
                self.request_redraw();
            }
        }
    }

    /// 粘贴文本/图片链接：有选区则替换，单次撤销事务；空串不产生编辑。
    pub(super) fn insert_paste(&mut self, text: &str) {
        let old_caret = self.editor.caret();
        if let Some(dirty) = self.editor.paste(text) {
            self.goal_x = None;
            self.invalidate_reveal(old_caret, self.editor.caret());
            self.after_edit(dirty);
        }
    }

    /// Home/End（及 Shift 扩选）：光标落到当前视觉行首/尾（`visual_line_edges` 语义）。
    pub(super) fn move_line_edge(&mut self, start: bool, extend: bool) {
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
        let Some(l) = self.layouts.get(&id) else {
            return;
        };
        let (s, e) = self.engine.visual_line_edges(l, caret - range.start);
        let target = range.start + if start { s } else { e };
        let old_caret = self.editor.caret();
        let moved = if extend {
            self.editor.set_caret_extend(target)
        } else {
            self.editor.set_caret(target)
        };
        if !moved {
            return;
        }
        self.goal_x = None; // 水平移动：重置目标列
        self.invalidate_reveal(old_caret, target);
        self.request_redraw();
    }

    pub(super) fn move_vertical(&mut self, delta: i32, extend: bool) {
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
            let Some(l) = self.layouts.get(&id) else {
                return;
            };
            let (cx, cy, ch) = self.engine.caret_xy(l, caret - range.start);
            (cx, cy, ch, l.height)
        };
        let goal = *self.goal_x.get_or_insert(cx);
        // 目标：本块上/下一视觉行；越界则上/下一块的末/首行
        let target_y_local = cy + if delta < 0 { -1.0 } else { ch };
        let target = if target_y_local >= 0.0 && target_y_local < lh {
            let l = &self.layouts[&id];
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
                (g.id, g.kind, g.range)
            };
            // 目标相邻块同样按需补齐（含表格网格/数学图）
            self.ensure_block(nid);
            if nkind == BlockKind::Table && self.table_shows_grid(nid, &nrange) {
                let Some(t) = self.tables.get(&nid) else {
                    return;
                };
                t.hit(goal, if delta < 0 { t.height - 1.0 } else { 0.0 })
            } else {
                let Some(nl) = self.layouts.get(&nid) else {
                    return;
                };
                let y = if delta < 0 { nl.height - 1.0 } else { 0.0 };
                nrange.start + self.engine.hit(nl, goal, y).min(nrange.len())
            }
        };
        let old_caret = self.editor.caret();
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.invalidate_reveal(old_caret, target);
        self.scroll_to_caret();
        self.request_redraw();
    }

    /// 块内物理坐标 → 文档偏移：表格网格态走单元格命中（落单元格内容起点），
    /// 其余走文本布局命中。
    pub(super) fn hit_block(
        &self,
        id: BlockId,
        kind: BlockKind,
        range: &veil_core::Range,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        if kind == BlockKind::Table && self.table_shows_grid(id, range) {
            return self.tables.get(&id).map(|t| t.hit(x, y));
        }
        let l = self.layouts.get(&id)?;
        Some(range.start + self.engine.hit(l, x, y).min(range.len()))
    }

    /// 鼠标位置 → 文档偏移（块间空白归入下一块起始；视口顶部之上归首块）。
    pub(super) fn hit_offset(&mut self) -> Option<usize> {
        let vp = self.viewport();
        let x = (self.mouse.x as f32 - MARGIN * self.scale).max(0.0);
        let y = (self.mouse.y as f32 - vp.y + self.scroll_y).max(0.0);
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let g = blocks.iter().rev().find(|g| g.top <= y).or_else(|| blocks.first())?;
            (g.id, g.kind, g.range, g.top)
        };
        // 命中块可能尚未布局：先按需补齐再读缓存
        self.ensure_block(id);
        self.hit_block(id, kind, &range, x, (y - top).max(0.0))
    }

    /// 落点更新 caret。`extend` = 保留 anchor（拖选/Shift+点击）。
    pub(super) fn point_caret(&mut self, extend: bool) {
        let Some(target) = self.hit_offset() else {
            return;
        };
        let old_caret = self.editor.caret();
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.invalidate_reveal(old_caret, target);
        self.goal_x = None;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 执行菜单动作。返回 true = 退出应用。
    pub(super) fn perform_menu(&mut self, action: MenuAction) -> bool {
        match action {
            MenuAction::Open => self.open_via_dialog(),
            MenuAction::Save => self.save_now(),
            MenuAction::SaveAs => self.save_as(),
            MenuAction::Export => self.export_now(),
            MenuAction::Undo => {
                let old = self.editor.caret();
                if let Some(dirty) = self.editor.apply(Intent::Undo) {
                    self.goal_x = None;
                    self.invalidate_reveal(old, self.editor.caret());
                    self.after_edit(dirty);
                }
            }
            MenuAction::Redo => {
                let old = self.editor.caret();
                if let Some(dirty) = self.editor.apply(Intent::Redo) {
                    self.goal_x = None;
                    self.invalidate_reveal(old, self.editor.caret());
                    self.after_edit(dirty);
                }
            }
            MenuAction::Cut => self.cut_selection(),
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Paste => self.paste_clipboard(),
            MenuAction::ThemeSystem => self.set_theme_mode(ThemeMode::System),
            MenuAction::ThemeLight => self.set_theme_mode(ThemeMode::Light),
            MenuAction::ThemeDark => self.set_theme_mode(ThemeMode::Dark),
            MenuAction::Quit => return true,
        }
        self.request_redraw();
        false
    }

    pub(super) fn copy_selection(&mut self) {
        if let Some(t) = self.editor.copy() {
            self.clipboard.set(&t);
        }
    }

    pub(super) fn cut_selection(&mut self) {
        let old_caret = self.editor.caret();
        if let Some((t, dirty)) = self.editor.cut() {
            self.clipboard.set(&t);
            self.goal_x = None;
            self.invalidate_reveal(old_caret, self.editor.caret());
            self.after_edit(dirty);
        }
    }

    /// 左键按下路由（菜单 > 菜单栏 > 滚动条 > 状态栏 > 正文光标）。
    /// 返回 true = 退出应用。菜单打开时点击外部只关菜单、不移动光标。
    pub(super) fn on_left_click(&mut self) -> bool {
        let (mx, my) = (self.mouse.x as f32, self.mouse.y as f32);
        let scale = self.scale;
        let vp = self.viewport();
        if let Some(open) = self.chrome.open {
            if let Some(item) = chrome::hit_popup_item(mx, my, open, scale) {
                self.chrome.close();
                if let Some(action) = chrome::action_at(open, item) {
                    return self.perform_menu(action);
                }
                return false;
            }
            if chrome::popup_contains(mx, my, open, scale) {
                return false; // 弹窗内边距：吞掉点击
            }
            self.chrome.close();
            if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
                if m != open {
                    self.chrome.open = Some(m);
                }
                self.request_redraw();
                return false;
            }
            self.request_redraw();
            return false;
        }
        if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
            self.chrome.toggle(m);
            self.request_redraw();
            return false;
        }
        let doc_h = self.doc_height();
        if chrome::scrollbar_hit(vp, doc_h, mx, my, scale) {
            if let Some((ty, th)) = chrome::scroll_thumb(vp, doc_h, self.scroll_y, scale) {
                if my >= ty && my < ty + th {
                    self.chrome.scroll_drag = Some(my - ty);
                } else {
                    let dir = if my < ty { -1 } else { 1 };
                    self.scroll_y = chrome::page_scroll(vp, doc_h, self.scroll_y, dir);
                    self.set_ime_area();
                    self.request_redraw();
                }
            }
            return false;
        }
        if my < vp.y || my >= vp.y + vp.h {
            return false; // 状态栏等 chrome 区域：不落光标
        }
        self.dragging = true;
        self.point_caret(self.mods.shift);
        false
    }

    /// 悬停高亮（菜单标题 / 弹出菜单项）。
    pub(super) fn update_hover(&mut self) {
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        let menu = chrome::hit_menu_bar(x, y, self.scale);
        let item = self
            .chrome
            .open
            .and_then(|open| chrome::hit_popup_item(x, y, open, self.scale).map(|i| (open, i)));
        if self.chrome.hover_menu != menu || self.chrome.hover_item != item {
            self.chrome.hover_menu = menu;
            self.chrome.hover_item = item;
            self.request_redraw();
        }
    }
}

/// 节点（含嵌套子节点）激活态在两次光标位置间是否变化（Reveal 失效判定）。
pub(super) fn activation_changed(n: &InlineNode, old: usize, new: usize) -> bool {
    node_active(n.range, old) != node_active(n.range, new)
        || n.children.iter().any(|c| activation_changed(c, old, new))
}

/// 光标在引用块内的行号（块外 None）：按 `\n` 计数，行数规则与排版一致（剥唯一尾换行）。
pub(super) fn quote_line_of(text: &str, range: &veil_core::Range, p: usize) -> Option<usize> {
    if p < range.start || p > range.end {
        return None;
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    let rel = (p - range.start).min(body.len());
    Some(body[..rel].bytes().filter(|&b| b == b'\n').count())
}

/// winit 键 → 内部 `Key`。ctrl 按住时 text 为空，字母键退化为 Z/Y 供快捷键判定。
pub(super) fn map_key(event: &winit::event::KeyEvent) -> Option<Key> {
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
        _ => None,
    }
}
