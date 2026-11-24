use crate::app::{App, ImagePreview};
use crate::chrome::{self, MenuAction};
use crate::editor::Intent;
use crate::geometry::BASE_LINE_HEIGHT;
use crate::i18n::{clip_error, t, tf, Key as I18nKey};
use crate::input::{Action, Key};
use crate::interaction::{footnote_def_id, mermaid_body_is_empty, mermaid_live_open_notice};
use crate::paint::{
    active_cell_index, table_cell_start, table_delete_column, table_delete_row,
    table_insert_column, table_insert_row, table_set_align,
};
use crate::theme::ThemeMode;
use crate::view::ctx_menu::{CtxAction, CtxKind, TableCtx};
use crate::view::{block_details_spans, ctx_menu};
use std::path::Path;
use veil_core::{
    heading_slug, heading_slugs, BlockId, BlockKind, CellAlign, HtmlTag, InlineKind, InlineNode,
    Range,
};

impl App {
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
            allow_remote_images: self.config.allow_remote_images,
            can_save: self.editor.is_dirty() || self.doc.path.is_none(),
            can_export: self.editor.session().text_len() > 0,
            can_undo: self.editor.session().undo_depth() > 0,
            can_redo: self.editor.session().redo_depth() > 0,
        }
    }

    /// 导出/打印的统一入口守卫：菜单、快捷键、模态重试与任何命令分发都必须
    /// 先经过这里，空文档时与菜单禁用态一致地 no-op 并给出提示。
    pub(crate) fn output_allowed(&mut self) -> bool {
        if self.menu_flags().can_export {
            return true;
        }
        self.ui_state
            .chrome
            .notify_error(t(I18nKey::NoticeOutputEmptyDocument));
        self.request_redraw();
        false
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
            // 可停靠项：跳过分隔线与只读提示
            let nav: Vec<usize> = self
                .ui_state
                .ctx
                .items()
                .iter()
                .enumerate()
                .filter(|(_, (_, a))| ctx_menu::action_enabled(*a))
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
                if ctx_menu::action_enabled(action) {
                    self.ctx_exec(action);
                }
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
            MenuAction::About => self.show_about_modal(),
            MenuAction::ExportDiagnostics => self.export_diagnostics(),
            MenuAction::CheckUpdate => self.check_update(),
        }
        self.request_redraw();
        false
    }

    pub(crate) fn copy_selection(&mut self) {
        if let Some(t) = self.editor.copy() {
            self.ports.clipboard.set(&t);
        }
    }

    /// 复制某围栏代码块的源码（不含围栏行与语言标记）。
    /// 纯剪贴板/视图动作：不写文档、不开撤销步、不置脏。
    pub(crate) fn copy_code_block(&mut self, id: BlockId) {
        let Some((_, body)) = self.editor.session().fenced_code_source(id) else {
            return;
        };
        self.ports.clipboard.set(&body);
        // 就地成功反馈：该块按钮暂时显示「已复制」（鼠标移开或提示消退后复原）。
        self.window_state.code_copied = Some(id);
        self.ui_state.chrome.notice = t(I18nKey::NoticeCodeCopied).into();
        self.request_redraw();
    }

    /// 键盘路径（Ctrl+Shift+C）：复制光标所在围栏代码块；不在代码块内时给出提示。
    pub(crate) fn copy_code_block_at_caret(&mut self) {
        let off = self.editor.caret();
        let id = self
            .index
            .blocks
            .iter()
            .find(|g| {
                g.range.start <= off
                    && off <= g.range.end
                    && matches!(&g.kind, BlockKind::FencedCode(_))
            })
            .map(|g| g.id);
        match id {
            Some(id) => self.copy_code_block(id),
            None => {
                self.ui_state
                    .chrome
                    .notify_error(t(I18nKey::NoticeCodeBlockRequired));
                self.request_redraw();
            }
        }
    }

    /// 左键命中代码块复制按钮：返回块 id（先按 y 粗筛，避免对全部块求几何）。
    pub(crate) fn code_copy_hit(&self) -> Option<BlockId> {
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        let vp = self.viewport();
        self.index.blocks.iter().find_map(|g| {
            if !matches!(&g.kind, BlockKind::FencedCode(_)) {
                return None;
            }
            let line_h = self.engine.doc_line_height() * self.input_state.scale;
            let top = vp.y + g.top - self.window_state.scroll_y;
            if my < top || my >= top + line_h {
                return None;
            }
            let h = self.code_header_layout(g)?;
            h.button.contains(mx, my).then_some(g.id)
        })
    }

    /// 代码块复制按钮点击：消费点击，避免落到光标/选区路径。
    pub(crate) fn try_copy_code_block(&mut self) -> bool {
        let Some(id) = self.code_copy_hit() else {
            return false;
        };
        self.copy_code_block(id);
        true
    }

    /// 左键命中已渲染的图片位图：打开放大预览（纯视图态）。
    /// 源码态/打印态不拦截；光标进入图片节点后位图本就不绘制，无需额外判定。
    pub(crate) fn try_open_image_preview(&mut self) -> bool {
        if self.source_mode() || self.window_state.print.is_some() {
            return false;
        }
        let (mx, my) = (
            self.input_state.mouse.x as f32,
            self.input_state.mouse.y as f32,
        );
        let vp = self.viewport();
        if my < vp.y || my >= vp.y + vp.h {
            return false;
        }
        self.ensure_layouts_visible();
        let content_x = self.content_x();
        let scroll = self.window_state.scroll_y;
        let hit = self.index.blocks.iter().find_map(|g| {
            let l = &self.cache.layouts.get(&g.id)?.value;
            let sy = vp.y + g.top - scroll;
            l.imgs.iter().find_map(|p| {
                let crate::view::InlineAsset::Image(key) = &p.key else {
                    return None;
                };
                let x = content_x + p.x;
                let y = sy + p.y;
                (mx >= x && mx < x + p.width as f32 && my >= y && my < y + p.height as f32)
                    .then(|| key.clone())
            })
        });
        let Some(key) = hit else {
            return false;
        };
        let title = Path::new(&key)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| key.clone());
        self.enter_exclusive_overlay();
        self.ui_state.image_preview = Some(ImagePreview {
            key,
            title,
            zoom: 1.0,
            pan: (0.0, 0.0),
        });
        self.window_state.dragging = false;
        self.request_redraw();
        true
    }

    /// 图片预览按键：Esc 关闭、+/-/0 缩放、方向键/翻页平移。
    /// 打开期间它是唯一输入接收者，正文文档与光标完全不变。
    pub(crate) fn image_preview_key(&mut self, key: &Key) {
        use crate::input::Key as K;
        // Esc 先处理：关闭需要独占 &mut self，不能与预览字段的借用重叠。
        if matches!(key, K::Escape) {
            self.close_image_preview();
            return;
        }
        let step = 40.0 * self.input_state.scale;
        let page = (self.viewport().h * 0.8).max(step);
        let Some(pv) = self.ui_state.image_preview.as_mut() else {
            return;
        };
        match key {
            K::Text(s) => match s.as_str() {
                "+" | "=" => pv.zoom = crate::print::next_zoom(pv.zoom, true),
                "-" | "_" => pv.zoom = crate::print::next_zoom(pv.zoom, false),
                "0" => {
                    pv.zoom = 1.0;
                    pv.pan = (0.0, 0.0);
                }
                _ => return,
            },
            K::Equal => pv.zoom = crate::print::next_zoom(pv.zoom, true),
            K::Minus => pv.zoom = crate::print::next_zoom(pv.zoom, false),
            K::Digit0 => {
                pv.zoom = 1.0;
                pv.pan = (0.0, 0.0);
            }
            K::Left => pv.pan.0 -= step,
            K::Right => pv.pan.0 += step,
            K::Up => pv.pan.1 -= step,
            K::Down => pv.pan.1 += step,
            K::PageUp => pv.pan.1 -= page,
            K::PageDown => pv.pan.1 += page,
            K::Home => {
                pv.zoom = 1.0;
                pv.pan = (0.0, 0.0);
            }
            _ => return,
        }
        self.request_redraw();
    }

    /// 预览滚轮：Ctrl 缩放；普通滚轮上下平移。仅在预览打开时由事件层调用。
    pub(crate) fn image_preview_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        use winit::event::MouseScrollDelta;
        let scale = self.input_state.scale;
        let dy = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * 3.0 * BASE_LINE_HEIGHT * scale,
            MouseScrollDelta::PixelDelta(p) => p.y as f32,
        };
        let Some(pv) = self.ui_state.image_preview.as_mut() else {
            return;
        };
        if self.input_state.mods.ctrl {
            pv.zoom = crate::print::next_zoom(pv.zoom, dy > 0.0);
        } else {
            pv.pan.1 += dy;
        }
        self.request_redraw();
    }

    /// 关闭图片预览（不触碰文档/光标/撤销栈）。
    pub(crate) fn close_image_preview(&mut self) {
        if self.ui_state.image_preview.take().is_some() {
            self.request_redraw();
        }
    }

    /// 右键命中的普通围栏代码块（mermaid 有专用菜单，不归此路径）。
    fn plain_code_block_at(&self, off: usize) -> Option<BlockId> {
        self.index
            .blocks
            .iter()
            .find(|g| {
                g.range.start <= off
                    && off <= g.range.end
                    && matches!(
                        &g.kind,
                        BlockKind::FencedCode(l)
                            if !l.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("mermaid"))
                    )
            })
            .map(|g| g.id)
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
    pub(crate) fn ctx_layout(&self) -> ctx_menu::Layout {
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
        let (kind, link, mermaid_body, insert_start, mermaid_occurrence) = if in_sel {
            (CtxKind::Selection, None, None, None, None)
        } else {
            self.editor.set_caret(off);
            self.window_state.goal_x = None;
            // 优先级：图片（单元格内也命中）→ 表格内 link → table_ctx →
            // mermaid 围栏块（带专用菜单）→ 块级 link → 普通块
            // 图片与链接共用同一份 inline 解析（原实现各解析一次整块）。
            let nodes = self.inlines_at_offset(off);
            match nodes.as_deref().and_then(|n| self.image_at_in(n, off)) {
                Some((path, missing)) => {
                    if missing {
                        // 缺失图片：菜单打开时给出可见提示（DEF-T67-E1-005）。
                        self.ui_state.chrome.notify_error(
                            crate::i18n::t(crate::i18n::Key::NoticeImageMissing)
                                .replace("{}", &path),
                        );
                    }
                    (CtxKind::Image { path, missing }, None, None, None, None)
                }
                None => match nodes.as_deref().and_then(|n| self.link_at_in(n, off)) {
                    Some(url) => (CtxKind::Link, Some(url), None, None, None),
                    None => match self.table_ctx_at(off) {
                        Some(tc) => (CtxKind::Table(tc), None, None, None, None),
                        None => match self.plain_code_block_at(off) {
                            Some(id) => (CtxKind::CodeBlock { id }, None, None, None, None),
                            None => match self.mermaid_block_info_at(off) {
                                Some((body, body_start, _end)) if mermaid_body_is_empty(&body) => {
                                    // 空块右键也把光标落到 body 起点：模板项只在这里有意义，
                                    // 落在点击位置会破坏围栏结构。
                                    self.editor.set_caret(body_start);
                                    let occurrence = self.mermaid_occurrence_at(off);
                                    (
                                        CtxKind::MermaidEmptyBlock,
                                        None,
                                        Some(body),
                                        Some(body_start),
                                        Some(occurrence.unwrap_or(0)),
                                    )
                                }
                                Some((body, _start, _end)) => {
                                    let occurrence = self.mermaid_occurrence_at(off);
                                    (CtxKind::MermaidBlock, None, Some(body), None, occurrence)
                                }
                                None => {
                                    // Details 块优先于普通块：右键菜单给折叠/展开项。
                                    // B3：命中 details 子块时同样归到所属头块，菜单行为一致。
                                    let details_kind = self
                                        .index
                                        .blocks
                                        .iter()
                                        .find(|g| g.range.start <= off && off <= g.range.end)
                                        .and_then(|g| {
                                            let id = if matches!(&g.kind, BlockKind::Details { .. })
                                            {
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
                                    (
                                        details_kind.unwrap_or(CtxKind::Block),
                                        None,
                                        None,
                                        None,
                                        None,
                                    )
                                }
                            },
                        },
                    },
                },
            }
        };
        self.enter_exclusive_overlay();
        self.ui_state.ctx.open_at(mx, my, kind, link, mermaid_body);
        self.ui_state.ctx.mermaid_insert_start = insert_start;
        self.ui_state.ctx.mermaid_occurrence = mermaid_occurrence;
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
    fn mermaid_block_info_at(&self, off: usize) -> Option<(String, usize, usize)> {
        let g = self.index.blocks.iter().find(|g| {
            g.range.start <= off
                && off <= g.range.end
                && (matches!(
                    &g.kind,
                    BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
                ) || g.kind == BlockKind::Mermaid)
        })?;
        let (_, body) = self.editor.session().fenced_code_source(g.id)?;
        let src = self.editor.session().text_slice(g.range)?;
        let first_nl = src.find('\n')?;
        Some((body, g.range.start + first_nl + 1, g.range.end))
    }

    /// 左键命中空 mermaid 块：光标落到该块 body 起点，并在鼠标处打开仅含
    /// 三条模板项的菜单。返回 true 表示本次点击已被消费。
    pub(crate) fn try_open_mermaid_empty_menu(&mut self) -> bool {
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
                    && (matches!(
                        &g.kind,
                        BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")
                    ) || g.kind == BlockKind::Mermaid)
            })
            .is_some_and(|g| y >= g.top && y < g.top + self.block_height(g));
        if !inside {
            return false;
        }
        let Some((body, body_start, _end)) = self.mermaid_block_info_at(off) else {
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
        self.enter_exclusive_overlay();
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
        let Some((id, range, kind, is_block_details)) = self
            .index
            .blocks
            .iter()
            .find(|g| !self.block_hidden(g) && g.range.start <= off && off < g.range.end)
            .map(|g| {
                (
                    g.id,
                    g.range,
                    g.kind.clone(),
                    matches!(g.kind, BlockKind::Details { .. }),
                )
            })
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
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let spans = block_details_spans(&kind, body, &nodes, range);
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
        let (id, range, kind, hidden) = {
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
            (g.id, g.range, g.kind.clone(), self.block_hidden(g))
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
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let mut changed = false;
        for s in block_details_spans(&kind, body, &nodes, range) {
            let body = (range.start + s.body.0, range.start + s.body.1);
            if body.0 <= caret && caret < body.1 {
                changed |= self.ui_state.expanded_inline_details.insert((id, s.key));
            }
        }
        if changed {
            self.recompute_geometry();
        }
    }

    /// REN-SWEEP-001：Front Matter 的「非活动隐藏 / 光标进入显源码」是纯视图态，
    /// 不参与布局缓存键；光标进入或离开时触发一次几何重算，让块高在 0 与源码高
    /// 之间切换（块字节与解析结果不变）。
    pub(crate) fn sync_frontmatter_at_caret(&mut self) {
        let caret = self.effective_caret();
        let revealed = self.index.blocks.iter().any(|g| {
            self.block_is_frontmatter(g) && g.range.start <= caret && caret <= g.range.end
        });
        if revealed != self.ui_state.frontmatter_revealed {
            self.ui_state.frontmatter_revealed = revealed;
            self.index.geometry_stale = true;
        }
    }

    /// Details 块折叠/展开：命中 Details 块 → 根据展开态和 y 位置决定是否切换。
    /// 收起态：整块可视区（仅 summary 一行）点击都触发展开；
    /// 展开态：只 summary 行（y ∈ [top, top + line_h)）触发收起，正文区落光标。
    /// 返回 true 表示本次点击已被折叠逻辑消费（不走后续 point_caret）。
    pub(crate) fn try_toggle_details_on_click(&mut self) -> bool {
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

    /// 点击生成目录（`[TOC]`）的某一行 → 跳到对应标题；源码态或光标在块内时不拦截。
    /// 目录文本是视图派生（`toc_display_text` 一行一条目），文档字节不受点击影响。
    pub(crate) fn try_jump_toc_on_click(&mut self) -> bool {
        if self.source_mode() || self.window_state.print.is_some() {
            return false;
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
        if !g.toc_marker {
            return false;
        }
        let (id, start, end) = (g.id, g.range.start, g.range.end);
        let caret = self.effective_caret();
        if start <= caret && caret <= end {
            return false; // 光标在块内：显示 `[TOC]` 源码，按普通编辑处理
        }
        self.ensure_block(id);
        let entry = {
            let Some(m) = self.cache.layouts.get(&id) else {
                return false;
            };
            if !m.value.toc_rendered {
                return false;
            }
            m.value.line_at_layout(off.saturating_sub(start))
        };
        self.jump_to_toc_entry(entry)
    }

    /// 目录第 `entry` 行对应的标题（跳过空标题，与 `toc_entries` 同源）→ 落光标并滚动。
    fn jump_to_toc_entry(&mut self, entry: usize) -> bool {
        let mut seen = 0usize;
        let target = self.index.blocks.iter().find_map(|g| {
            if !matches!(
                g.kind,
                BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
            ) {
                return None;
            }
            let (_, text) = self.editor.session().heading(g.id)?;
            if text.is_empty() {
                return None;
            }
            let hit = seen == entry;
            seen += 1;
            hit.then_some(g.range.start)
        });
        let Some(off) = target else {
            return false;
        };
        if !self.editor.set_caret(off) {
            return false;
        }
        self.window_state.goal_x = None;
        self.scroll_to_caret();
        self.request_redraw();
        true
    }

    /// 命中偏移所在块的 inline 节点（表格按单元格取，其余取块 inlines）。
    pub(crate) fn inlines_at_offset(&self, offset: usize) -> Option<Vec<InlineNode>> {
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
    pub(crate) fn link_at(&self, offset: usize) -> Option<String> {
        let nodes = self.inlines_at_offset(offset)?;
        self.link_at_in(&nodes, offset)
    }

    /// 在已解析的 inline 节点中取链接 URL：悬停/右键共用同一份解析结果。
    pub(crate) fn link_at_in(&self, nodes: &[InlineNode], offset: usize) -> Option<String> {
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
        let url = find(nodes, offset)?.url?;
        self.editor.session().text_slice(url)
    }

    /// 命中偏移处的图片源与缺失状态（DEF-T67-E1-005）。
    /// 支持 Markdown `![alt](src)` 与白名单自闭合 `<img src="…">`；
    /// 远程/内联图片视为“存在”，本地图片按文档目录解析后探测文件。
    /// 测试辅助入口：命中偏移处的图片源与缺失状态。
    #[cfg(test)]
    pub(crate) fn image_at(&self, offset: usize) -> Option<(String, bool)> {
        let nodes = self.inlines_at_offset(offset)?;
        self.image_at_in(&nodes, offset)
    }

    /// 在已解析的 inline 节点中取图片源与缺失状态（右键与悬停共用单次解析）。
    fn image_at_in(&self, nodes: &[InlineNode], offset: usize) -> Option<(String, bool)> {
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
        let node = find(nodes, offset)?;
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

    /// 在已解析的 inline 节点中取 `<abbr title="…">` 文本（悬停单次解析共用）。
    /// 空 title 不产出提示（HTML 语义上等同于无提示）。
    pub(crate) fn abbr_title_in(nodes: &[InlineNode], offset: usize) -> Option<&str> {
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
        find(nodes, offset)
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
    pub(crate) fn follow_footnote_ref(&mut self, offset: usize) -> bool {
        let Some(id) = self.footnote_ref_id_at(offset) else {
            return false;
        };
        let def = self.editor.session().footnote_definition(&id);
        match def {
            Some(def) => self.jump_to_offset(def.start),
            None => {
                self.ui_state
                    .chrome
                    .notify_error(t(I18nKey::NoticeFootnoteDefMissing));
                self.request_redraw();
            }
        }
        true
    }

    /// Ctrl+单击定义块末尾 `↩` → 该 id 的首个引用（`↩` 为零宽插入段，锚在块末）。
    pub(crate) fn follow_footnote_back(&mut self, offset: usize) -> bool {
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
                self.ui_state
                    .chrome
                    .notify_error(t(I18nKey::NoticeFootnoteRefMissing));
                self.request_redraw();
            }
        }
        true
    }

    /// Ctrl+单击 `[文本](#slug)` → 同文档标题起点。
    /// slug 先按导出口径（`heading_slugs` 去重结果）原样匹配，未命中再按
    /// `heading_slug` 归一化匹配，容忍用户手写 `#Hello World` 这类非导出值。
    pub(crate) fn jump_to_anchor(&mut self, url: &str) {
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
                self.ui_state
                    .chrome
                    .notify_error(t(I18nKey::NoticeAnchorMissing));
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
            self.ui_state
                .chrome
                .notify_error(tf(I18nKey::NoticeLinkUnsupported, &[&target]));
            self.request_redraw();
            return;
        }
        if let Err(e) = self.ports.url_opener.open_url(&target) {
            self.ui_state
                .chrome
                .notify_error(tf(I18nKey::NoticeLinkFailed, &[&e.to_string()]));
        }
        self.request_redraw();
    }

    /// 执行右键菜单动作（菜单随后由调用方关闭；CopyLink 载荷读自 ctx.link）。
    pub(crate) fn ctx_exec(&mut self, action: CtxAction) {
        match action {
            CtxAction::Separator => {}
            CtxAction::Cut => self.cut_selection(),
            CtxAction::Copy => self.copy_selection(),
            CtxAction::CopyCodeBlock => {
                if let Some(id) = self.ui_state.ctx.code_id {
                    self.copy_code_block(id);
                }
            }
            CtxAction::Paste => self.paste_clipboard(),
            CtxAction::SelectAll => self.perform_action(Action::Doc(Intent::SelectAll)),
            // 与主菜单「加粗/斜体/链接」同一 Intent，不重复实现格式化逻辑。
            CtxAction::ToggleBold => self.perform_action(Action::Doc(Intent::ToggleWrap("**"))),
            CtxAction::ToggleItalic => self.perform_action(Action::Doc(Intent::ToggleWrap("*"))),
            CtxAction::InsertLink => self.perform_action(Action::Doc(Intent::InsertLink)),
            CtxAction::CopyLink => {
                if let Some(u) = self.ui_state.ctx.link.clone() {
                    // 复制出去的是可直接使用的目标：`www.` 补全协议；
                    // 非白名单 scheme 与打开同口径拒绝，不把危险地址带出程序。
                    let target = crate::normalize_link_url(&u);
                    if veil_core::link_url_is_safe(&target) {
                        self.ports.clipboard.set(&target);
                    } else {
                        self.ui_state
                            .chrome
                            .notify_error(tf(I18nKey::NoticeLinkUnsupported, &[&target]));
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
            // Mermaid：预览 / 复制源码（读 ctx.mermaid_body）
            CtxAction::MermaidPreview | CtxAction::MermaidCopySource => {
                let Some(body) = self.ui_state.ctx.mermaid_body.clone() else {
                    return;
                };
                // 「在 mermaid.live 打开」是显式外部动作：只复制源码并交系统浏览器
                // 打开站点首页，URL 不携带源码；产品自身不上传、不抓取。
                self.ports.clipboard.set(&body);
                if action == CtxAction::MermaidPreview {
                    let result = self.ports.url_opener.open_url("https://mermaid.live/");
                    if let Some(notice) = mermaid_live_open_notice(result) {
                        self.ui_state.chrome.notify_error(notice);
                        self.request_redraw();
                    }
                }
            }
            // Mermaid 手动预览/快照只走进程内本地渲染，不改写活源码。
            CtxAction::MermaidPreviewImage | CtxAction::MermaidInsertImage => {
                let Some(body) = self.ui_state.ctx.mermaid_body.clone() else {
                    return;
                };
                let insert = matches!(action, CtxAction::MermaidInsertImage);
                let dark = self.input_state.theme.dark;
                let title = veil_core::detect_type(&body)
                    .unwrap_or("mermaid")
                    .to_string();
                let source_hash = crate::app::content_hash(body.as_bytes());
                let occurrence = self.ui_state.ctx.mermaid_occurrence;

                let label = format!(
                    "mermaid-manual:{:016x}",
                    crate::app::content_hash(format!("{body}\0{dark}").as_bytes())
                );
                match self
                    .ports
                    .mermaid
                    .render_blocking(label.clone(), body, dark)
                {
                    Ok(rendered) => {
                        let img = crate::image::ImageData {
                            pixels: rendered.data,
                            width: rendered.width,
                            height: rendered.height,
                        };
                        if insert {
                            self.insert_mermaid_snapshot(&img, source_hash, occurrence);
                        } else {
                            self.cache.remote_failed.remove(&label);
                            self.cache.insert_remote_image(label.clone(), img);
                            self.input_state.revs.remote_images += 1;
                            self.ui_state.image_preview = Some(ImagePreview {
                                key: label,
                                title,
                                zoom: 1.0,
                                pan: (0.0, 0.0),
                            });
                            self.request_redraw();
                        }
                    }
                    Err(e) => {
                        if !insert {
                            self.cache.remote_failed.insert(label);
                        }
                        self.ui_state.chrome.notify_error(tf(
                            I18nKey::NoticeMermaidRenderFailed,
                            &[&clip_error(&e)],
                        ));
                        self.request_redraw();
                    }
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
                // 图片文件可能已被替换：内容代次递增使引用该图的块布局键失配，
                // 重排后块高/占位按新自然尺寸刷新（仅清位图缓存不够）。
                self.input_state.revs.image_content += 1;
                self.ensure_layouts_visible();
                self.recompute_geometry();
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
}
