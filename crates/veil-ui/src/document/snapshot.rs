//! 版本历史编排（N1/B8a）：打开/只读预览/恢复/删除/清空。
//!
//! 预览复用正文渲染器：打开预览时暂存实时编辑器，把 `self.editor` 换成快照文本的
//! 只读 Editor；正文区因此仍走同一条排版/绘制路径，不再造第二套渲染器。
//! 恢复只写编辑器缓冲区（单撤销事务 + 标脏），绝不直接覆盖磁盘。

use crate::app::{content_hash, App, ErrorPurpose, PendingIntent};
use crate::editor::Editor;
use crate::geometry::Rect;
use crate::i18n::{t, tf, Key};
use crate::view::history::{self as view, HistoryAction};

impl App {
    /// 版本历史面板几何（内容视口 → Rect）。
    pub(crate) fn history_layout(&self) -> view::Layout {
        let vp = self.viewport();
        view::layout(
            Rect {
                x: vp.x,
                y: vp.y,
                w: vp.w,
                h: vp.h,
            },
            self.input_state.scale,
        )
    }

    /// 实时文档文本：预览期间实时编辑器暂存在历史状态里。
    pub(crate) fn history_live_text(&self) -> String {
        self.history_live_editor().session().serialize()
    }

    fn history_live_editor(&self) -> &Editor {
        self.ui_state
            .history
            .saved_editor
            .as_ref()
            .unwrap_or(&self.editor)
    }

    /// 实时文档脏态（预览期间不能看预览 Editor 的脏标）。
    pub(crate) fn history_live_dirty(&self) -> bool {
        self.history_live_editor().is_dirty()
    }

    /// 菜单/快捷键入口：已开则关，未开则开。
    pub(crate) fn toggle_history(&mut self) {
        if self.ui_state.history.open {
            self.close_history();
        } else {
            self.open_history();
        }
    }

    /// 打开版本历史：关闭其它浮层，按稳定键读取快照列表（时间倒序）。
    pub(crate) fn open_history(&mut self) {
        if self.ui_state.history.open {
            return;
        }
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        self.close_quick();
        self.close_search();
        self.editor.clear_preedit();
        self.ui_state.history.open = true;
        self.ui_state.history.saved_editor = None;
        self.ui_state.history.selected = None;
        self.history_reload_list();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 关闭面板并归还实时编辑器（预览只读，不改文档字节）。
    pub(crate) fn close_history(&mut self) {
        if !self.ui_state.history.open && self.ui_state.history.saved_editor.is_none() {
            return;
        }
        self.history_end_preview();
        self.ui_state.history.open = false;
        self.ui_state.history.clear();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 重新读取列表；失败进入错误态（空列表 + 可读原因），不崩溃。
    fn history_reload_list(&mut self) {
        let key = self.doc.history_key;
        let loaded = self.ports.snapshot.list_snapshots(key);
        self.history_apply_entries(loaded);
    }

    fn history_apply_entries(&mut self, loaded: Result<Vec<crate::app::SnapshotMeta>, String>) {
        match loaded {
            Ok(entries) => {
                let labels = entries
                    .iter()
                    .map(|m| self.ports.clock.local_datetime(m.ts_ms))
                    .collect();
                self.ui_state.history.entries = entries;
                self.ui_state.history.labels = labels;
                self.ui_state.history.selected = None;
                self.ui_state.history.error = None;
            }
            Err(e) => {
                self.ui_state.history.entries.clear();
                self.ui_state.history.labels.clear();
                self.ui_state.history.selected = None;
                self.ui_state.history.error = Some(e);
            }
        }
        self.ui_state.history.current_hash = content_hash(self.history_live_text().as_bytes());
        self.ui_state.history.scroll = 0.0;
        self.ui_state.history.hover = None;
    }

    /// 选中并预览第 `index` 份快照（只读）：正文区切到快照文本。
    pub(crate) fn history_select(&mut self, index: usize) {
        let Some(meta) = self.ui_state.history.entries.get(index).copied() else {
            return;
        };
        let key = self.doc.history_key;
        match self.ports.snapshot.read_snapshot(key, meta.ts_ms) {
            Ok(text) => {
                self.history_begin_preview(&text);
                self.ui_state.history.selected = Some(index);
                let panel = view::panel_rect(
                    Rect {
                        x: self.viewport().x,
                        y: self.viewport().y,
                        w: self.viewport().w,
                        h: self.viewport().h,
                    },
                    self.input_state.scale,
                );
                self.ui_state.history.scroll = view::ensure_visible(
                    self.ui_state.history.scroll,
                    index,
                    self.ui_state.history.entries.len(),
                    panel,
                    self.input_state.scale,
                );
                self.ui_state.history.error = None;
            }
            Err(e) => {
                // 损坏/缺失快照：保留原预览，状态栏给出可读原因，不崩溃
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeSnapshotReadFailed, &[&e]));
            }
        }
        self.request_redraw();
    }

    /// 键盘上下移动选中项；首次按键从最新版本开始。
    pub(crate) fn history_step(&mut self, delta: i32) {
        self.ui_state.history.step(delta);
        if let Some(index) = self.ui_state.history.selected {
            self.history_select(index);
        }
    }

    /// 预览换入/换出：`self.editor` 只承载只读展示，实时编辑器完整暂存（含撤销栈）。
    fn history_begin_preview(&mut self, text: &str) {
        if self.ui_state.history.saved_editor.is_none() {
            let saved = std::mem::replace(
                &mut self.editor,
                Editor::new_with_callouts(text, &self.callouts),
            );
            let h = &mut self.ui_state.history;
            h.saved_editor = Some(saved);
            h.saved_scroll = self.window_state.scroll_y;
            h.saved_goal = self.window_state.goal_y;
        } else {
            self.editor = Editor::new_with_callouts(text, &self.callouts);
        }
        self.window_state.scroll_y = 0.0;
        self.window_state.goal_y = 0.0;
        self.window_state.goal_x = None;
        self.input_state.revs.session += 1;
        self.refresh_index();
    }

    /// 归还实时编辑器并恢复滚动位置（幂等：无预览时 no-op）。
    fn history_end_preview(&mut self) {
        let Some(saved) = self.ui_state.history.saved_editor.take() else {
            return;
        };
        self.editor = saved;
        self.window_state.scroll_y = self.ui_state.history.saved_scroll;
        self.window_state.goal_y = self.ui_state.history.saved_goal;
        self.window_state.goal_x = None;
        self.input_state.revs.session += 1;
        self.refresh_index();
    }

    /// 恢复当前选中版本：先退出预览（让保存/未保存确认作用在实时编辑器上）。
    pub(crate) fn history_restore_selected(&mut self) {
        let Some(index) = self.ui_state.history.selected else {
            self.ui_state.chrome.notice = t(Key::NoticeSelectVersion).into();
            self.request_redraw();
            return;
        };
        let Some(meta) = self.ui_state.history.entries.get(index).copied() else {
            return;
        };
        let key = self.doc.history_key;
        let text = match self.ports.snapshot.read_snapshot(key, meta.ts_ms) {
            Ok(t) => t,
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeSnapshotReadFailed, &[&e]));
                self.request_redraw();
                return;
            }
        };
        self.history_end_preview();
        if self.editor.is_dirty() {
            // 既有未保存确认弹窗：保存/不保存/取消；结论落地后才执行恢复
            self.begin_unsaved(PendingIntent::RestoreSnapshot(text));
            return;
        }
        self.history_apply_restore(text);
    }

    /// 恢复落地：整篇替换进缓冲区（单撤销事务 + 标脏），关闭面板并提示可撤销。
    pub(crate) fn history_apply_restore(&mut self, text: String) {
        self.history_end_preview();
        if let Some(dirty) = self.editor.replace_all_text(&text) {
            self.after_edit(dirty);
        }
        self.ui_state.history.open = false;
        self.ui_state.history.clear();
        self.ui_state.chrome.notice = t(Key::NoticeRestoredVersion).into();
        self.set_ime_area();
        self.request_redraw();
    }

    /// 删除选中快照：危险操作二次确认（B6 错误态危险样式）。
    pub(crate) fn history_request_delete(&mut self) {
        let Some(index) = self.ui_state.history.selected else {
            self.ui_state.chrome.notice = t(Key::NoticeSelectVersionToDelete).into();
            self.request_redraw();
            return;
        };
        let Some(meta) = self.ui_state.history.entries.get(index).copied() else {
            return;
        };
        let label = self
            .ui_state
            .history
            .labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| crate::app::format_unix_ms_utc(meta.ts_ms));
        self.show_error_modal(
            t(Key::ModalDeleteSnapshotTitle),
            tf(
                Key::ModalDeleteSnapshotBody,
                &[&label, &view::format_size(meta.bytes)],
            ),
            ErrorPurpose::HistoryDelete(meta.ts_ms),
            vec![
                crate::view::modal::ModalButton::new(
                    1,
                    t(Key::BtnDelete),
                    crate::view::modal::ButtonRole::Danger,
                )
                .with_mnemonic('d'),
                crate::view::modal::ModalButton::new(
                    3,
                    t(Key::BtnCancel),
                    crate::view::modal::ButtonRole::Secondary,
                ),
            ],
        );
    }

    /// 删除落地（确认后）：若删的是正在预览的版本，先归还实时编辑器。
    pub(crate) fn history_delete(&mut self, ts_ms: u64) {
        let selected_ts = self
            .ui_state
            .history
            .selected
            .and_then(|i| self.ui_state.history.entries.get(i))
            .map(|m| m.ts_ms);
        if selected_ts == Some(ts_ms) {
            self.history_end_preview();
        }
        let key = self.doc.history_key;
        match self.ports.snapshot.delete_snapshot(key, ts_ms) {
            Ok(()) => {
                self.ui_state.chrome.notice = t(Key::NoticeDeletedSnapshot).into();
                let loaded = self.ports.snapshot.list_snapshots(key);
                self.history_apply_entries(loaded);
                // 尝试保持原选中（删除非选中项时预览不丢）
                if let Some(ts) = selected_ts.filter(|ts| *ts != ts_ms) {
                    if let Some(i) = self
                        .ui_state
                        .history
                        .entries
                        .iter()
                        .position(|m| m.ts_ms == ts)
                    {
                        self.history_select(i);
                    }
                }
            }
            Err(e) => self
                .ui_state
                .chrome
                .notify_error(tf(Key::NoticeDeleteSnapshotFailed, &[&e])),
        }
        self.request_redraw();
    }

    /// 清空历史：二次确认（危险样式）。
    pub(crate) fn history_request_clear(&mut self) {
        let n = self.ui_state.history.entries.len();
        if n == 0 {
            self.ui_state.chrome.notice = t(Key::NoticeNoHistory).into();
            self.request_redraw();
            return;
        }
        self.show_error_modal(
            t(Key::ModalClearHistoryTitle),
            tf(Key::ModalClearHistoryBody, &[&n.to_string()]),
            ErrorPurpose::HistoryClear,
            vec![
                crate::view::modal::ModalButton::new(
                    1,
                    t(Key::BtnClear),
                    crate::view::modal::ButtonRole::Danger,
                )
                .with_mnemonic('d'),
                crate::view::modal::ModalButton::new(
                    3,
                    t(Key::BtnCancel),
                    crate::view::modal::ButtonRole::Secondary,
                ),
            ],
        );
    }

    /// 清空落地（确认后）。
    pub(crate) fn history_clear_all(&mut self) {
        self.history_end_preview();
        let key = self.doc.history_key;
        match self.ports.snapshot.clear_snapshots(key) {
            Ok(n) => {
                self.ui_state.chrome.notice = tf(Key::NoticeHistoryCleared, &[&n.to_string()]);
                let loaded = self.ports.snapshot.list_snapshots(key);
                self.history_apply_entries(loaded);
            }
            Err(e) => self
                .ui_state
                .chrome
                .notify_error(tf(Key::NoticeClearHistoryFailed, &[&e])),
        }
        self.request_redraw();
    }

    /// 面板按钮分派（绘制命中与键盘共用）。
    pub(crate) fn history_action(&mut self, action: HistoryAction) {
        match action {
            HistoryAction::Restore => self.history_restore_selected(),
            HistoryAction::Delete => self.history_request_delete(),
            HistoryAction::Clear => self.history_request_clear(),
            HistoryAction::Close => self.close_history(),
        }
    }
}
