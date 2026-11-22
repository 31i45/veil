//! 文档生命周期：打开/另存/保存（含冲突备份与原子落盘）、自动保存调度、
//! 外部变更轮询与标题派生。纯策略与 IO 编排，绘制与交互不在此文件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use veil_core::{HtmlTag, InlineKind, InlineNode};

use crate::app::{
    ActiveModal, App, ErrorPurpose, FileEvent, FileStore, ModalPurpose, PendingIntent, StartupError,
};
use crate::chrome::path_label;
use crate::docio::{decode, encode};
use crate::editor::Editor;
use crate::i18n::{clip_error, t, tf, Key};
use crate::mermaid_renderer::{MermaidImage, MermaidRenderer};
use crate::view::modal;

use super::{recovery, session, ExternalState};

/// 恢复写盘失败后的退避间隔（0b）：磁盘写失败多为持续状态，5s 重试避免忙轮询。
pub(crate) const RECOVERY_RETRY: Duration = Duration::from_secs(5);

/// 保存来源：自动保存绝不覆盖外部修改，手工保存经备份后允许覆盖。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveKind {
    Manual,
    Auto,
}

impl App {
    /// 把监听目标切换到当前路径（草稿则停止监听）。失败仅提示，不影响编辑。
    pub(crate) fn watch_current(&mut self) {
        let Some(path) = self.doc.path.clone() else {
            self.ports.watcher.unwatch();
            return;
        };
        if let Err(e) = self.ports.watcher.watch(&path) {
            self.ui_state
                .chrome
                .notify_error(tf(Key::NoticeWatchUnavailable, &[&e]));
            eprintln!("文件监听不可用 {}: {e}", path.display());
        }
    }

    /// 外部变更对外呈现的单一状态（状态栏/标题共用）。
    /// 待备份的新版本优先于「已备份」：备份登记只说明旧版本有副本，不能掩盖新版本。
    pub(crate) fn external_state(&self) -> ExternalState {
        self.persist_state.persist.status()
    }

    /// 窗口标题：路径派生，用文件名而非完整路径（所有专业编辑器惯例）。
    /// 脏标 `*` 前置与冲突提示后缀是 chrome UI 落地前的最小可见载体。
    /// 脏标必须前置：标题栏从右侧截断时末尾标记会被吃掉。
    pub(crate) fn title_text(&self) -> String {
        let base = match &self.doc.path {
            Some(p) => tf(
                Key::WindowTitle,
                &[p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(t(Key::UntitledDraft))],
            ),
            None => t(Key::AppTitle).to_string(),
        };
        let mut title = base;
        if self.editor.is_dirty() {
            title.insert_str(0, t(Key::TitleDirtyPrefix));
        }
        match self.external_state() {
            ExternalState::None => {}
            ExternalState::Changed => title.push_str(t(Key::TitleExternalChanged)),
            ExternalState::BackedUp => title.push_str(t(Key::TitleExternalBackedUp)),
            ExternalState::Missing => title.push_str(t(Key::TitleExternalMissing)),
        }
        title
    }

    pub(crate) fn update_title(&mut self) {
        let title = self.title_text();
        if self.window_state.last_title.as_deref() == Some(title.as_str()) {
            return; // 逐键 SetWindowText 会引入同步窗口消息开销，标题未变则跳过
        }
        if let Some(w) = &self.window_state.window {
            w.set_title(&title);
        }
        self.window_state.last_title = Some(title);
    }

    /// Ctrl+S：显式保存（允许在备份外部版本后覆盖）。
    pub(crate) fn save_now(&mut self) {
        self.save(SaveKind::Manual);
    }

    /// 序列化 → 编解码 → 冲突检测 → 经 FileStore 原子落盘。
    ///
    /// 冲突检测：磁盘现有字节 != 基线即视为被外部修改（设计文档 §B5「内容哈希对比」的
    /// 字节等价物）。此时先把磁盘版本写成同目录 `<stem>.conflict-<n><ext>` 备份，再分流：
    /// - 手工保存：显式意图，备份后覆盖本文件（外部版本不丢，标题提示）；
    /// - 自动保存：绝不覆盖，挂起自动保存并保持脏态，等用户显式保存；
    /// - 备份失败：取消保存（不允许无备份覆盖）；磁盘读不到（新建/被删）不算冲突。
    ///
    /// 成功清脏并更新基线；失败保持脏态（可读错误同时写状态栏 notice）。
    pub(crate) fn save(&mut self, kind: SaveKind) {
        let Some(path) = self.doc.path.clone() else {
            if kind == SaveKind::Manual {
                // 草稿无关联文件：与系统编辑器一致，保存即转「另存为」选位置
                self.save_as();
            }
            return;
        };
        let data = encode(&self.editor.session().serialize(), self.doc.bom);
        let external = match self.ports.store.load(&path) {
            Ok(disk) if disk != self.doc.baseline => Some(disk),
            _ => None,
        };
        if let Some(disk) = &external {
            // 同一外部版本只备份一次（自动冲突已写副本 → 手工保存不再重复）
            if self.persist_state.persist.registry() != Some(disk.as_slice()) {
                match self.write_conflict_copy(&path, disk) {
                    Ok(bp) => {
                        self.persist_state.persist.mark_backed_up(disk.clone());
                        eprintln!("检测到外部修改，原文件已备份到 {}", bp.display());
                    }
                    Err(e) => {
                        self.ui_state
                            .chrome
                            .notify_error(tf(Key::NoticeExternalBackupFailed, &[&e]));
                        eprintln!("外部修改备份失败，取消保存（不覆盖原文件）: {e}");
                        if kind == SaveKind::Manual {
                            self.show_error_modal(
                                t(Key::ModalSaveFailedTitle),
                                tf(
                                    Key::ModalExternalBackupFailBody,
                                    &[&path_label(Some(path.as_path())), &e],
                                ),
                                ErrorPurpose::Save,
                                save_error_buttons(),
                            );
                        }
                        self.update_title();
                        self.request_redraw();
                        return;
                    }
                }
            }
            // 否则登记已等于磁盘版本：提示态本就为「已备份」，无需再写副本
            if kind == SaveKind::Auto {
                self.persist_state.persist.set_hold(true);
                eprintln!("自动保存已暂停：{} 被外部修改", path.display());
                self.update_title();
                return;
            }
        }
        // 覆盖旧版本前留档（N1/B8a）：把即将被覆盖的磁盘版本写入稳定身份目录。
        // 同内容由存储层去重；基线为空（新文档）无旧版本可留。
        let archive_notice = if data != self.doc.baseline {
            self.archive_baseline()
        } else {
            None
        };
        match self.ports.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                self.doc.baseline = data;
                crate::app::record_diag("doc", "save ok");
                // 本会话最近一次成功保存的时刻（自动/手工同源）：状态栏「已保存 HH:MM」
                let hhmm = self.ports.clock.local_hhmm();
                self.persist_state.last_saved_hhmm = Some(hhmm);
                self.persist_state.persist.keep_notice_only(); // 备份登记作废，「已备份」提示保留
                self.persist_state.persist.clear_pending(); // 磁盘已是本版内容：待备份提示作废
                self.persist_state.persist.set_missing(false); // 删除/不可读状态由本次写入恢复
                if external.is_none() {
                    self.persist_state.persist.clear_backup(); // 本次无冲突（含手工覆盖后）→ 清提示
                }
                if kind == SaveKind::Manual {
                    self.persist_state.persist.set_hold(false);
                    self.ui_state.chrome.notice =
                        tf(Key::NoticeSaved, &[&path_label(Some(path.as_path()))]);
                } else if self.ui_state.chrome.notice == t(Key::NoticeCancelDirty) {
                    // 取消关窗的提示描述的是「仍有未保存修改」，随后成功的自动保存已让磁盘与
                    // 编辑器一致；此时保留该提示会自相矛盾，故仅清这一条，其他提示不动。
                    self.ui_state.chrome.notice.clear();
                }
                self.persist_state.persist.clear_activity(); // 已干净：取消自动保存调度
                                                             // 当前版本也入历史：列表可标「当前」，且恢复面板始终能看到刚保存的版本
                self.record_current_snapshot();
                // 留档失败在成功提示之后落地：正文/磁盘已保存，但历史不可静默缺失。
                if let Some(n) = archive_notice {
                    self.ui_state.chrome.notify_error(n);
                }
                self.clear_recovery(); // 磁盘已是最新：恢复副本作废（0b）
                self.update_title();
            }
            Err(e) => {
                // 失败不沉默：保持 dirty、不动 last_saved_hhmm，并给出可行动的下一步
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeSaveFailed, &[&e]));
                eprintln!("保存失败 {}: {e}", path.display());
                // 仅手工保存弹错误弹窗；自动保存失败留在状态栏，避免打断正在输入的用户。
                if kind == SaveKind::Manual {
                    self.show_error_modal(
                        t(Key::ModalSaveFailedTitle),
                        tf(
                            Key::ModalSaveFailedBody,
                            &[&path_label(Some(path.as_path())), &e],
                        ),
                        ErrorPurpose::Save,
                        save_error_buttons(),
                    );
                }
            }
        }
        self.request_redraw();
    }

    /// 保存成功后把当前内容也写入历史（同内容由存储层去重）。
    /// 失败不沉默（release 无控制台）：状态栏可见提示，正文与磁盘均不受影响。
    fn record_current_snapshot(&mut self) {
        let text = self.editor.session().serialize();
        if text.is_empty() {
            return;
        }
        let key = self.doc.history_key;
        if let Err(e) = self.ports.snapshot.save_snapshot(key, &text) {
            eprintln!("版本快照写入失败（key={key:016x}）: {e}");
            self.ui_state
                .chrome
                .notify_error(tf(Key::NoticeSnapshotFailed, &[&e]));
        }
    }

    /// 保存前留档：把当前基线（即将被覆盖的磁盘版本）写入文档稳定身份目录。
    /// 解码失败（非 UTF-8）或空内容不写入——快照是文本版本，不承担二进制备份。
    /// 返回失败提示文案（调用方在成功提示之后落地，避免被「已保存」覆盖）。
    pub(crate) fn archive_baseline(&mut self) -> Option<String> {
        if self.doc.baseline.is_empty() {
            return None;
        }
        if let Ok((text, _)) = decode(&self.doc.baseline) {
            if !text.is_empty() {
                let key = self.doc.history_key;
                if let Err(e) = self.ports.snapshot.save_snapshot(key, &text) {
                    eprintln!("版本快照写入失败（key={key:016x}）: {e}");
                    return Some(tf(Key::NoticeSnapshotFailed, &[&e]));
                }
            }
        }
        None
    }

    /// 打开/另存为到已存在目标时，覆盖前留档目标文件现有内容。
    /// 与 `archive_baseline` 同一策略：失败留痕（返回提示文案），不静默丢弃。
    fn archive_existing_target(&mut self, key: u64, bytes: &[u8]) -> Option<String> {
        if let Ok((text, _)) = decode(bytes) {
            if !text.is_empty() {
                if let Err(e) = self.ports.snapshot.save_snapshot(key, &text) {
                    eprintln!("版本快照写入失败（key={key:016x}）: {e}");
                    return Some(tf(Key::NoticeSnapshotFailed, &[&e]));
                }
            }
        }
        None
    }

    /// 把磁盘现存版本写到同目录冲突副本（首个未占用序号，经 FileStore 原子写）。
    pub(crate) fn write_conflict_copy(
        &mut self,
        path: &Path,
        disk: &[u8],
    ) -> Result<PathBuf, String> {
        let bp = conflict_copy_path(&mut *self.ports.store, path)
            .ok_or_else(|| t(Key::NoticeConflictNameExhausted).to_string())?;
        self.ports.store.save(&bp, disk)?;
        Ok(bp)
    }

    /// 自动保存判定：距上次内容编辑/自动保存尝试达静默期，且无路径/IME/冲突挂起阻碍。
    pub(crate) fn autosave_due(&self, now: Instant) -> bool {
        // 模态未决时不后台保存：用户正在阅读失败原因，不能悄悄清脏或再次失败。
        if self.ui_state.modal.is_some() {
            return false;
        }
        let Some(last) = self.persist_state.persist.activity() else {
            return false;
        };
        self.persist_state.autosave.due(
            self.editor.is_dirty(),
            self.doc.path.is_some(),
            self.editor.preedit().is_some(),
            self.persist_state.persist.hold(),
            now.saturating_duration_since(last),
        )
    }

    /// 尝试自动保存（事件循环空闲回调调用）。无论成败都推进计时起点，避免忙轮询。
    pub(crate) fn maybe_autosave(&mut self, now: Instant) {
        if !self.autosave_due(now) {
            return;
        }
        self.persist_state.persist.note_autosave_attempt(now);
        self.save(SaveKind::Auto);
        self.request_redraw();
    }

    /// 恢复写盘判定（0b）：脏且非 IME 组合期即候选（无路径草稿/冲突挂起也覆盖）；
    /// 正常路径按静默期（复用自动保存的 2s），上次失败后按退避节奏重试。
    /// 起点是「最后一次真实编辑」，自动保存尝试不得刷新它（DEF-T08-E1-001）。
    pub(crate) fn recovery_write_due(&self, now: Instant) -> bool {
        if !self.editor.is_dirty() || self.editor.preedit().is_some() {
            return false;
        }
        if let Some(t) = self.persist_state.recovery_failed_at {
            return now.saturating_duration_since(t) >= RECOVERY_RETRY;
        }
        self.persist_state
            .autosave
            .deadline(self.persist_state.persist.recovery_activity())
            .is_some_and(|at| now >= at)
    }

    /// 崩溃恢复写盘（0b）：把当前完整内容原子写入恢复槽。内容未变（哈希相同）不重写；
    /// 失败给 notice 并记录失败时刻（退避重试），绝不 panic——恢复是最后防线，不是新风险。
    pub(crate) fn maybe_recovery_write(&mut self, now: Instant) {
        if !self.recovery_write_due(now) {
            return;
        }
        let text = self.editor.session().serialize();
        let data = recovery::encode_recovery(self.doc.path.as_deref(), self.doc.bom, &text);
        let hash = recovery::fnv64(&data);
        if self.persist_state.recovery_hash == Some(hash) {
            self.persist_state.recovery_failed_at = None;
            self.persist_state.persist.clear_recovery_activity();
            return; // 磁盘已是当前内容快照
        }
        match self.ports.recovery.save(&data) {
            Ok(()) => {
                self.persist_state.recovery_hash = Some(hash);
                self.persist_state.recovery_failed_at = None;
                // 已写入当前内容：下一次真实编辑再重新计时，避免无内容变化时反复唤醒。
                self.persist_state.persist.clear_recovery_activity();
            }
            Err(e) => {
                self.persist_state.recovery_failed_at = Some(now);
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeRecoveryWriteFailed, &[&e]));
                eprintln!("崩溃恢复写入失败: {e}");
            }
        }
    }

    /// 保存/另存为成功后清除恢复副本（0b）。清除失败仅记录：残留文件在下次启动时
    /// 经「恢复内容 == 磁盘内容」判定自清，不会造成错误恢复。
    /// 多窗口共用一个最后写入者槽：清槽前核对指纹，避免删掉其他窗口较新的快照。
    pub(crate) fn clear_recovery(&mut self) {
        let Some(own_hash) = self.persist_state.recovery_hash else {
            return; // 本会话未写过恢复文件，磁盘上不会有属于本内容的副本
        };
        let owns_slot = self
            .ports
            .recovery
            .load()
            .is_ok_and(|data| recovery::fnv64(&data) == own_hash);
        if owns_slot {
            if let Err(e) = self.ports.recovery.clear() {
                eprintln!("恢复副本清理失败（下次启动将自清）: {e}");
            }
        }
        self.persist_state.recovery_hash = None;
        self.persist_state.recovery_failed_at = None;
        self.persist_state.persist.clear_recovery_activity();
        self.persist_state.snapshot_deadline = None;
    }

    /// 打开/另存为成功时登记最近文件并持久化（0b）。写失败不阻断主流程。
    pub(crate) fn note_opened(&mut self, path: PathBuf) {
        // 先合并磁盘最新顺序，再追加本窗口独有条目，最后提升本次路径。
        if let Ok(text) = self.ports.session.load() {
            let mut merged = session::parse_session(&text);
            for old in &self.persist_state.recent {
                if !merged.contains(old) {
                    merged.push(old.clone());
                }
            }
            self.persist_state.recent = merged;
        }
        session::touch_recent(&mut self.persist_state.recent, path);
        let text = session::encode_session(&self.persist_state.recent);
        if let Err(e) = self.ports.session.save(&text) {
            eprintln!("最近文件记录失败: {e}");
        }
    }

    /// 外部变更轮询（空闲回调调用）：只置状态与提示，绝不自动改文档内容或落盘。
    ///
    /// 检测：监听事件 → 读磁盘 → 与基线/已备份版本比较。备份仍只在保存覆盖前执行，
    /// 避免外部程序每保存一次就在文档旁积一份副本；同一版本/已备份版本不重复提示。
    /// 脏文档弹四出口选择框；干净文档静默重载（磁盘已是用户可见内容，无未保存损失）。
    pub(crate) fn poll_external(&mut self) {
        let events = self.ports.watcher.poll();
        if events.is_empty() {
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        let removed = events.contains(&FileEvent::Removed);
        match self.ports.store.load(&path) {
            Ok(disk) => {
                let recovered = self.persist_state.persist.missing();
                self.persist_state.persist.set_missing(false);
                if disk == self.doc.baseline {
                    // 外部内容回到基线：撤销「待备份」状态（已备份提示保留至下次保存）
                    let had_pending = self.persist_state.persist.clear_pending();
                    if had_pending && !self.persist_state.persist.backed_up() {
                        self.persist_state.persist.set_hold(false);
                    }
                    if recovered {
                        self.ui_state.chrome.notice = tf(
                            Key::NoticeFileRestored,
                            &[&path_label(Some(path.as_path()))],
                        );
                        self.update_title();
                        self.request_redraw();
                    }
                    return;
                }
                if self.persist_state.persist.registry() == Some(disk.as_slice())
                    || self.persist_state.persist.pending() == Some(disk.as_slice())
                {
                    return; // 已备份 / 同一外部版本：不重复提示
                }
                if !self.editor.is_dirty() {
                    // 干净文档无未保存损失：静默重载，避免无意义的模态打断
                    self.reload_external(disk, &path);
                    return;
                }
                self.persist_state.persist.begin_external(disk); // 脏文档绝不静默覆盖外部版本
                self.ui_state.chrome.notice = t(Key::NoticeExternalChanged).into();
                self.update_title();
                self.request_redraw();
                self.show_external_modal(&path);
            }
            Err(_) => {
                if self.persist_state.persist.missing() {
                    return;
                }
                self.persist_state.persist.set_missing(true);
                self.persist_state.persist.clear_pending();
                if !self.persist_state.persist.backed_up() {
                    self.persist_state.persist.set_hold(false); // 重建文件不构成覆盖，允许自动保存
                }
                self.ui_state.chrome.notice = if removed {
                    t(Key::NoticeExternalDeleted).into()
                } else {
                    t(Key::NoticeExternalUnreadable).into()
                };
                self.update_title();
                self.request_redraw();
            }
        }
    }

    /// 脏文档外部修改的唯一弹窗装配点：选择前不改内容/磁盘，仅保留已置的 pending+hold。
    /// 已有模态未决时不叠加（避免覆盖用户的未保存决定），只保留状态栏提示与挂起。
    fn show_external_modal(&mut self, path: &Path) {
        if self.ui_state.modal.is_some() {
            return;
        }
        self.close_lite_overlays();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(t(Key::UntitledDraft));
        let spec = modal::ModalSpec::new(
            tf(Key::ModalExternalChangedTitle, &[name]),
            t(Key::ModalExternalChangedBody),
            vec![
                modal::ModalButton::new(1, t(Key::BtnReload), modal::ButtonRole::Danger)
                    .with_mnemonic('r'),
                modal::ModalButton::new(2, t(Key::BtnKeepCurrent), modal::ButtonRole::Primary)
                    .with_mnemonic('k'),
                modal::ModalButton::new(3, t(Key::BtnSaveAs), modal::ButtonRole::Secondary)
                    .with_mnemonic('a'),
                modal::ModalButton::new(4, t(Key::BtnCancel), modal::ButtonRole::Secondary),
            ],
        );
        let state = modal::ModalState::new(&spec);
        self.ui_state.modal = Some(ActiveModal {
            spec,
            state,
            intent: None,
            purpose: ModalPurpose::External,
        });
        self.sync_ime_for_modal();
        self.request_redraw();
    }

    /// 用磁盘字节整体替换编辑器（干净文档静默重载与「重新加载」共用）。
    /// 解码失败绝不改内容：提示不可读并保留既有 pending/hold；返回是否重载成功。
    fn reload_external(&mut self, disk: Vec<u8>, path: &Path) -> bool {
        let (text, bom) = match decode(&disk) {
            Ok(x) => x,
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(t(Key::NoticeExternalUnreadable));
                eprintln!("外部版本解码失败 {}: {e}", path.display());
                return false;
            }
        };
        self.editor = Editor::new_with_callouts(&text, &self.callouts);
        self.doc.baseline = disk;
        self.doc.bom = bom;
        self.persist_state.persist.reset(); // 清 pending/hold/missing/backup/activity
        self.clear_recovery();
        self.input_state.revs.session += 1;
        self.input_state.revs.image_base += 1;
        // 整篇替换：远程图片位图/失败记忆/pending 属文档会话，全部清空避免串档。
        self.cache.clear_remote();
        self.ui_state.hover_link = None;
        // 表格列宽覆盖按块 id 生效：整篇替换即整体失效（纯视图态，无副作用）
        self.ui_state.tables.widths.clear();
        self.ui_state.tables.drag = None;
        self.refresh_index();
        if self.ui_state.outline.open {
            self.refresh_outline();
        }
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.ui_state.chrome.notice = tf(Key::NoticeExternalReloaded, &[&path_label(Some(path))]);
        self.update_title();
        self.set_ime_area();
        self.request_redraw();
        true
    }

    /// 导出/打印前置：把缓存缺失的 Mermaid 块同步渲染补齐（有预算上限）。
    /// 后台请求可能还在飞、caret 在块内被跳过、长文档未轮到——这些块在导出时
    /// 会退化成源码围栏，与「三端同源」不符，故在入口处补一次阻塞渲染。
    /// 失败静默：导出继续走已有的源码围栏降级（不打断用户）。
    pub(crate) fn ensure_mermaid_for_export(&mut self) {
        // 先收集缺失块，避免持有 session 借用时改 cache
        let missing: Vec<(veil_core::BlockId, String)> = {
            let session = self.editor.session();
            let mut out = Vec::new();
            for (id, kind, _range) in session.blocks() {
                if kind != veil_core::BlockKind::Mermaid {
                    continue;
                }
                if self.cache.mermaid_imgs.contains_key(&id) {
                    continue;
                }
                // 围栏内容（不含 ``` 行），与自动预览取源一致
                let body = session
                    .fenced_code_source(id)
                    .map(|(_, body)| body)
                    .unwrap_or_default();
                out.push((id, body));
            }
            out
        };
        if missing.is_empty() {
            return;
        }
        let dark = self.input_state.theme.dark;
        let started = std::time::Instant::now();
        // 预算上限：最多 16 次实际渲染 / 总耗时 10s，避免病态文档卡死导出。
        // 超限跳过的块不计数，避免大源码块把后续小块饿死。
        let mut rendered = 0usize;
        for (id, source) in missing {
            if rendered >= 16 || started.elapsed() > std::time::Duration::from_secs(10) {
                break;
            }
            // 与 trigger_mermaid_auto 相同上限：> 128 KiB 跳过
            if source.len() > 128 * 1024 {
                continue;
            }
            rendered += 1;
            let label = format!(
                "mermaid-export:{:016x}",
                crate::app::content_hash(format!("{source}\0{dark}").as_bytes())
            );
            if let Ok(img) = self.ports.mermaid.render_blocking(label, source, dark) {
                self.cache.mermaid_imgs.insert(id, img);
            }
        }
    }

    /// PDF/统一打印前置：填充本次会话独立的 light Mermaid map。
    /// 屏幕为 dark 时忽略 dark 屏幕缓存并强制离线 light 重渲；屏幕为 light 时复用已有图。
    /// 无论哪种主题都不写屏幕/HTML cache，失败保持缺失以沿用源码围栏降级。
    pub(crate) fn ensure_mermaid_for_print(
        &mut self,
        print_imgs: &mut HashMap<veil_core::BlockId, MermaidImage>,
    ) {
        let blocks: Vec<(veil_core::BlockId, String)> = {
            let session = self.editor.session();
            session
                .blocks()
                .into_iter()
                .filter_map(|(id, kind, _range)| {
                    if kind != veil_core::BlockKind::Mermaid {
                        return None;
                    }
                    let body = session
                        .fenced_code_source(id)
                        .map(|(_, body)| body)
                        .unwrap_or_default();
                    Some((id, body))
                })
                .collect()
        };
        ensure_mermaid_for_print_blocks(
            blocks,
            self.input_state.theme.dark,
            &self.cache.mermaid_imgs,
            print_imgs,
            &mut *self.ports.mermaid,
        );
    }

    /// Ctrl+E：经注入的 Exporter 导出 HTML。导出是显式副作用：无论文档是否有关联路径，
    /// 都先弹保存对话框确认输出位置（建议路径 = 同目录同名 `.html`，草稿为裸名，
    /// 由平台层补「文档」目录）。取消即中止，不产生文件；失败走 notice 不崩溃。
    /// 导出不清脏标（源文件未变）。
    pub(crate) fn export_now(&mut self) {
        if !self.output_allowed() {
            return;
        }
        let suggested = match self.doc.path.as_ref() {
            Some(p) => p.with_extension("html").to_string_lossy().into_owned(),
            None => t(Key::ExportUntitledHtml).to_string(),
        };
        let Some(target) = self.ports.dialogs.save_export(&suggested, "html") else {
            self.ui_state.chrome.notice = t(Key::NoticeExportCancelled).into();
            self.request_redraw();
            return;
        };
        self.ensure_mermaid_for_export();
        // 克隆 mermaid 渲染结果（导出低频，克隆可接受）。
        let mermaids: std::collections::HashMap<_, _> = self
            .cache
            .mermaid_imgs
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let mermaid_ref = if mermaids.is_empty() {
            None
        } else {
            Some(&mermaids)
        };
        match self.ports.exporter.export_html(
            self.editor.session(),
            self.doc.export_style,
            self.doc.path.as_deref(),
            Some(&target),
            mermaid_ref,
        ) {
            Ok(p) => {
                crate::app::record_diag("doc", "export ext=html");
                self.ui_state.chrome.notice = tf(Key::NoticeExportedHtml, &[&path_label(Some(&p))]);
                eprintln!("已导出 {}", p.display());
            }
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeExportFailed, &[&e]));
                eprintln!("导出失败: {e}");
                self.show_error_modal(
                    t(Key::ModalExportFailedTitle),
                    tf(
                        Key::ModalExportFailedBody,
                        &[&path_label(Some(&target)), &e],
                    ),
                    ErrorPurpose::ExportHtml,
                    retry_cancel_buttons(),
                );
            }
        }
        self.request_redraw();
    }

    /// Ctrl+Shift+S：另存为。写入成功才切换关联路径/基线并清脏；失败保留旧路径与脏标。
    /// 新路径改变图片相对基准：图片代次递增，图片/布局/表格按因果键失配重取。
    pub(crate) fn save_as(&mut self) {
        // 传完整路径：对话框直接定位到文档所在目录（只传裸名会落到系统记忆的任意目录）。
        let suggested = match self.doc.path.as_ref() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => t(Key::ExportUntitledMarkdown).to_string(),
        };
        let Some(path) = self.ports.dialogs.save_markdown(&suggested) else {
            self.ui_state.chrome.notice = t(Key::NoticeSaveAsCancelled).into();
            self.request_redraw();
            return;
        };
        self.save_as_path(path);
        self.request_redraw();
    }

    /// 另存为的落盘部分（已确定目标路径）：供对话框入口与错误弹窗「重试」共用。
    /// 成功才切换关联路径/基线并清脏；失败保留旧路径与脏标。
    fn save_as_path(&mut self, path: PathBuf) {
        let data = encode(&self.editor.session().serialize(), self.doc.bom);
        let was_unnamed = self.doc.path.is_none();
        let old_key = self.doc.history_key;
        let new_key = super::history::doc_key(&path);
        // 目标文件已存在：覆盖前把现有内容留档到目标路径键目录（失败留痕）
        let archive_notice = if let Ok(existing) = self.ports.store.load(&path) {
            if existing != data {
                self.archive_existing_target(new_key, &existing)
            } else {
                None
            }
        } else {
            None
        };
        match self.ports.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                let old_path = self.doc.path.clone();
                self.doc.path = Some(path.clone());
                self.doc.baseline = data;
                crate::app::record_diag("doc", "save ok");
                // 未命名草稿首次保存：临时会话键的历史迁移到路径键。
                // 迁移失败不得静默：旧历史保留在旧键（仍可在版本历史面板访问），
                // 且明确告知用户「历史未迁移」，不能只报「已另存为」（DEF-T08-001）。
                let mut history_notice = None;
                if was_unnamed && old_key != new_key {
                    match self.ports.snapshot.migrate_key(old_key, new_key) {
                        Ok(_) => self.doc.history_key = new_key,
                        Err(e) => {
                            self.doc.history_key = old_key;
                            eprintln!("版本历史迁移失败（旧历史保留，key={old_key:016x}）: {e}");
                            history_notice = Some(tf(
                                Key::NoticeHistoryMigrationFailed,
                                &[&path_label(Some(path.as_path())), &e],
                            ));
                        }
                    }
                } else {
                    self.doc.history_key = new_key;
                }
                self.record_current_snapshot();
                let hhmm = self.ports.clock.local_hhmm();
                self.persist_state.last_saved_hhmm = Some(hhmm);
                self.persist_state.persist.reset();
                self.input_state.revs.image_base += 1; // 图片相对基准变化：相关派生缓存键失配
                self.clear_recovery(); // 旧路径的恢复副本已随另存失效（0b）
                self.note_opened(path.clone()); // 登记最近文件（0b）
                self.ui_state.chrome.notice = tf(Key::NoticeSavedAs, &[&path_label(Some(&path))]);
                // 监听目标随关联路径切换；失败提示覆盖成功提示
                self.watch_current();
                // 跨目录另存：迁移相对图片资产，避免新位置断图（DEF-T08-002）。
                // 资产缺失/复制失败时给可见降级，不静默断链。
                let asset_notice = match old_path.as_deref() {
                    Some(old) => self.migrate_assets_for_save_as(old, &path),
                    None => None,
                };
                if let Some(n) = asset_notice.or(history_notice).or(archive_notice) {
                    self.ui_state.chrome.notify_error(n);
                }
                self.update_title();
            }
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeSaveAsFailed, &[&e]));
                eprintln!("另存为失败 {}: {e}", path.display());
                self.show_error_modal(
                    t(Key::ModalSaveAsFailedTitle),
                    tf(
                        Key::ModalSaveAsFailedBody,
                        &[&path_label(Some(path.as_path())), &e],
                    ),
                    ErrorPurpose::SaveAs(path),
                    vec![
                        modal::ModalButton::new(1, t(Key::BtnRetry), modal::ButtonRole::Primary)
                            .with_mnemonic('r'),
                        modal::ModalButton::new(
                            2,
                            t(Key::BtnChangeLocation),
                            modal::ButtonRole::Secondary,
                        ),
                        modal::ModalButton::new(3, t(Key::BtnCancel), modal::ButtonRole::Secondary),
                    ],
                );
            }
        }
    }

    /// 跨目录另存后的相对图片资产迁移（DEF-T08-002）：同目录无需处理；
    /// 新目录不同则把文档引用的相对图片复制到新目录的同一相对位置。
    /// 返回可见降级提示（有资产缺失/复制失败时），成功返回 None。
    fn migrate_assets_for_save_as(&mut self, old: &Path, new: &Path) -> Option<String> {
        let (Some(old_dir), Some(new_dir)) = (old.parent(), new.parent()) else {
            return None;
        };
        if super::history::normalize_doc_path(old_dir)
            == super::history::normalize_doc_path(new_dir)
        {
            return None;
        }
        let urls = self.relative_image_urls();
        if urls.is_empty() {
            return None;
        }
        let report = self.ports.assets.migrate_relative_assets(old, new, &urls);
        if report.incomplete() == 0 {
            return None;
        }
        eprintln!(
            "另存为后图片资产未全部迁移（{} 个）：缺失 {}，失败 {}",
            report.incomplete(),
            report.missing,
            report.failed.len()
        );
        Some(tf(
            Key::NoticeAssetsMigrationIncomplete,
            &[&path_label(Some(new)), &report.incomplete().to_string()],
        ))
    }

    /// 文档内全部「相对路径」图片 URL（Markdown 图片 + 行内 `<img src>`）。
    /// 绝对路径/远程 URL/带 scheme 的地址不参与迁移（位置与文档目录无关）。
    fn relative_image_urls(&self) -> Vec<String> {
        let session = self.editor.session();
        let mut out = Vec::new();
        for (id, _, _) in session.blocks() {
            if let Some(nodes) = session.block_inlines(id) {
                collect_image_urls(&nodes, session, &mut out);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// 打开失败：完整路径 + 原因 + 可操作出口（重试 / 重新选择 / 取消）。
    fn show_open_error(&mut self, path: &Path, reason: &str) {
        self.show_error_modal(
            t(Key::ModalOpenFailedTitle),
            tf(Key::ModalOpenFailedBody, &[&path_label(Some(path)), reason]),
            ErrorPurpose::Open(path.to_path_buf()),
            vec![
                modal::ModalButton::new(1, t(Key::BtnRetry), modal::ButtonRole::Primary)
                    .with_mnemonic('r'),
                modal::ModalButton::new(2, t(Key::BtnReselect), modal::ButtonRole::Secondary),
                modal::ModalButton::new(3, t(Key::BtnCancel), modal::ButtonRole::Secondary),
            ],
        );
    }

    /// 已有模态未决时错误不叠加，但也绝不静默丢弃：把标题、原因与恢复提示写进状态栏，
    /// 用户处理完当前弹窗后仍知道刚才的操作失败了。
    fn note_error_while_modal_open(&mut self, title: &str, message: &str) {
        let reason = message
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("原因：")
                    .or_else(|| line.trim().strip_prefix("原因:"))
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| message.lines().next().unwrap_or("").trim());
        self.ui_state.chrome.notify_error(tf(
            Key::NoticeErrorWhileModalOpen,
            &[title, &clip_error(reason)],
        ));
        self.request_redraw();
    }

    /// 关于弹窗：复用应用内模态。任何按钮都只关闭弹窗；主按钮（id=1）
    /// 额外经注入的 UrlOpener 打开官方 Release 页，失败只写通知。
    pub(crate) fn show_about_modal(&mut self) {
        if self.ui_state.modal.is_some() {
            return; // 已有模态未决：不叠加
        }
        self.close_lite_overlays();
        let config_dir = crate::theme::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "不可用".to_string());
        let log_dir = crate::theme::data_dir()
            .map(|d| d.join("logs").display().to_string())
            .unwrap_or_else(|| "不可用".to_string());
        let message = format!(
            "Veil {}（构建 {}）\n许可证：MIT\n\
             隐私：无遥测、无后台更新检查、无后台网络请求；远程图片默认关闭。\n\
             配置目录：{}\n诊断目录：{}\n官方来源：{}\n\
             第三方组件：Mermaid、QuickJS、linkedom（及全部 Rust 依赖）；完整许可见随包 THIRD-PARTY-LICENSES.md。",
            crate::diag::APP_VERSION,
            crate::diag::BUILD_COMMIT,
            config_dir,
            log_dir,
            crate::app::RELEASES_URL,
        );
        let spec = modal::ModalSpec::new(
            t(Key::AboutTitle),
            message,
            vec![
                modal::ModalButton::new(1, t(Key::AboutOpenHomepage), modal::ButtonRole::Primary),
                modal::ModalButton::new(2, t(Key::BtnClose), modal::ButtonRole::Secondary),
            ],
        );
        let state = modal::ModalState::new(&spec);
        self.ui_state.modal = Some(ActiveModal {
            spec,
            state,
            intent: None,
            purpose: ModalPurpose::About,
        });
        self.sync_ime_for_modal();
        self.request_redraw();
    }

    /// 关于弹窗结论：任何按钮都关闭（模态已被取出），id=1 额外打开官方主页。
    fn resolve_about(&mut self, id: u16) {
        if id == 1 {
            if let Err(e) = self.ports.url_opener.open_url(crate::app::RELEASES_URL) {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeLinkFailed, &[&e]));
            }
        }
        self.request_redraw();
    }

    /// 「检查更新」：仅用默认浏览器打开官方 Release 页；不产生进程内网络请求。
    pub(crate) fn check_update(&mut self) {
        if let Err(e) = self.ports.url_opener.open_url(crate::app::RELEASES_URL) {
            self.ui_state
                .chrome
                .notify_error(tf(Key::NoticeUpdateOpenFailed, &[&e]));
        }
        self.request_redraw();
    }

    /// 「导出诊断信息…」：显式保存对话框 → 白名单字段文本 → 原子写。
    /// 取消无副作用；成功/失败都写状态栏通知，任何失败只通知不 panic。
    pub(crate) fn export_diagnostics(&mut self) {
        let suggested = format!("veil-diagnostics-{}.txt", crate::diag::APP_VERSION);
        let Some(path) = self.ports.dialogs.save_text(&suggested, "txt") else {
            return; // 用户取消：不落盘、不改文档状态
        };
        let extra = self.diagnostics_extra();
        let text =
            crate::diag::render_diagnostics(std::env::consts::OS, std::env::consts::ARCH, &extra);
        match self.ports.store.save(&path, text.as_bytes()) {
            Ok(()) => {
                crate::app::record_diag("diag", "export ok");
                self.ui_state.chrome.notify(
                    tf(
                        Key::NoticeDiagnosticsSaved,
                        &[&path_label(Some(path.as_path()))],
                    ),
                    crate::chrome::NoticeKind::Success,
                );
            }
            Err(e) => {
                crate::app::record_diag("diag", "export err");
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeDiagnosticsFailed, &[&e]));
            }
        }
        self.request_redraw();
    }

    /// 诊断补充信息白名单：只允许开关/计数/枚举，不含完整路径、文件名或正文。
    fn diagnostics_extra(&self) -> Vec<(&'static str, String)> {
        let yes_no = |v: bool| if v { "yes" } else { "no" }.to_string();
        let config_exists = crate::theme::config_path().is_some_and(|p| p.exists());
        let chars = self.editor.session().serialize().chars().count();
        vec![
            ("配置存在", yes_no(config_exists)),
            ("文档已关联", yes_no(self.doc.path.is_some())),
            ("未保存修改", yes_no(self.editor.is_dirty())),
            ("文档字符数", chars.to_string()),
            ("主题", self.input_state.theme.mode.as_str().to_string()),
            ("允许远程图片", yes_no(self.config.allow_remote_images)),
            (
                "界面缩放",
                format!("{:.0}%", self.input_state.scale * 100.0),
            ),
            ("崩溃报告数", count_crash_reports().to_string()),
        ]
    }

    /// 错误弹窗的唯一装配点：与确认弹窗共用卡片/按钮/间距令牌，只在语义上标记危险。
    /// 已有模态未决时不叠加（避免覆盖未保存确认而丢失用户意图），改为写入状态栏提示。
    pub(crate) fn show_error_modal(
        &mut self,
        title: &str,
        message: String,
        purpose: ErrorPurpose,
        buttons: Vec<modal::ModalButton>,
    ) {
        if self.ui_state.modal.is_some() {
            self.note_error_while_modal_open(title, &message);
            return;
        }
        self.close_lite_overlays();
        let spec = modal::ModalSpec::error(title, message, buttons);
        let state = modal::ModalState::new(&spec);
        self.ui_state.modal = Some(ActiveModal {
            spec,
            state,
            intent: None,
            purpose: ModalPurpose::Error(purpose),
        });
        self.sync_ime_for_modal();
        self.request_redraw();
    }

    /// 启动即失败（CLI/IPC 打开失败）：首帧弹出与应用内其它错误弹窗同源的模态，
    /// 出口为「重试 / 打开其他文件 / 关闭」；关闭（含 Esc）即退出本窗口。
    pub(crate) fn show_startup_error(&mut self, err: StartupError) {
        if self.ui_state.modal.is_some() {
            self.note_error_while_modal_open(&err.title, &err.message);
            return;
        }
        self.close_lite_overlays();
        let path = err.path;
        let spec = modal::ModalSpec::error(
            err.title,
            err.message,
            vec![
                modal::ModalButton::new(1, t(Key::BtnRetry), modal::ButtonRole::Primary)
                    .with_mnemonic('r'),
                modal::ModalButton::new(2, t(Key::BtnOpenOther), modal::ButtonRole::Secondary)
                    .with_mnemonic('o'),
                modal::ModalButton::new(3, t(Key::BtnClose), modal::ButtonRole::Secondary),
            ],
        );
        let state = modal::ModalState::new(&spec);
        self.ui_state.modal = Some(ActiveModal {
            spec,
            state,
            intent: None,
            purpose: ModalPurpose::Error(ErrorPurpose::Startup(path)),
        });
        self.sync_ime_for_modal();
        self.request_redraw();
    }

    /// 未保存确认的挂起入口：干净立即执行意图并返回 true；
    /// 脏则登记模态（不阻塞）并返回 false，结论由 `resolve_modal` 驱动。
    pub(crate) fn begin_unsaved(&mut self, intent: PendingIntent) -> bool {
        if self.ui_state.modal.is_some() {
            return false; // 已有模态未决：不叠加
        }
        if !self.editor.is_dirty() {
            self.execute_intent(intent);
            return true;
        }
        self.close_lite_overlays();
        let name = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(t(Key::UntitledDraft));
        let spec = modal::ModalSpec::new(
            tf(Key::ModalUnsavedTitle, &[name]),
            t(Key::ModalUnsavedBody),
            vec![
                modal::ModalButton::new(1, t(Key::BtnSave), modal::ButtonRole::Primary)
                    .with_mnemonic('s'),
                modal::ModalButton::new(2, t(Key::BtnDontSave), modal::ButtonRole::Danger)
                    .with_mnemonic('c'),
                modal::ModalButton::new(3, t(Key::BtnCancel), modal::ButtonRole::Secondary),
            ],
        );
        let state = modal::ModalState::new(&spec);
        self.ui_state.modal = Some(ActiveModal {
            spec,
            state,
            intent: Some(intent),
            purpose: ModalPurpose::Unsaved,
        });
        self.sync_ime_for_modal();
        self.request_redraw();
        false
    }

    /// 执行被挂起的意图（不再二次确认）。
    pub(crate) fn execute_intent(&mut self, intent: PendingIntent) {
        match intent {
            PendingIntent::Close => {
                self.clear_recovery();
                self.window_state.exit_requested = true;
            }
            PendingIntent::New => self.new_draft_now(),
            PendingIntent::Open => self.open_via_dialog_now(),
            PendingIntent::OpenRecent(p) => self.open_path_now(p),
            PendingIntent::RestoreSnapshot(text) => self.history_apply_restore(text),
        }
    }

    /// 模态结论落地：未保存确认为 1=保存、2=不保存、3=取消；
    /// 错误弹窗为 1=重试、2=次要出口（另存/换位置/重新选择）、3=取消。
    pub(crate) fn resolve_modal(&mut self, id: u16) {
        let Some(active) = self.ui_state.modal.take() else {
            return;
        };
        self.sync_ime_for_modal();
        let known = active.spec.find(id).is_some();
        match active.purpose {
            ModalPurpose::Unsaved => self.resolve_unsaved(id, active.intent),
            ModalPurpose::External => self.resolve_external(id),
            ModalPurpose::About => self.resolve_about(id),
            ModalPurpose::Error(purpose) => {
                if !known {
                    self.ui_state.chrome.notice = t(Key::NoticeCancelled).into();
                } else {
                    match (purpose, id) {
                        // 保存失败：重试写同一路径
                        (ErrorPurpose::Save, 1) => self.save_now(),
                        // 保存失败：另存为（第二出口）
                        (ErrorPurpose::Save, 2) => self.save_as(),
                        // 另存为失败：重试写同一目标（不静默换目标）
                        (ErrorPurpose::SaveAs(path), 1) => self.save_as_path(path),
                        (ErrorPurpose::SaveAs(_), 2) => self.save_as(),
                        // 打开失败：重试读同一路径
                        (ErrorPurpose::Open(path), 1) => self.open_path_now(path),
                        // 打开失败：重新选择文件
                        (ErrorPurpose::Open(_), 2) => self.open_via_dialog_now(),
                        // 启动即失败：重试读同一路径；换文件；其余（含 Esc/「关闭」）退出本窗口
                        (ErrorPurpose::Startup(path), 1) => self.open_path_now(path),
                        (ErrorPurpose::Startup(_), 2) => self.open_via_dialog_now(),
                        (ErrorPurpose::Startup(_), _) => self.window_state.exit_requested = true,
                        // 导出失败：重走动作为用户重新确认输出位置
                        (ErrorPurpose::ExportHtml, 1) => self.export_now(),
                        (ErrorPurpose::ExportPdf, 1) => self.export_pdf(),
                        (ErrorPurpose::Print, 1) => self.print_document(),
                        // 版本历史危险确认：1 = 删除/清空
                        (ErrorPurpose::HistoryDelete(ts), 1) => self.history_delete(ts),
                        (ErrorPurpose::HistoryClear, 1) => self.history_clear_all(),
                        (purpose, _) => {
                            // 3 = 取消 / 未知 id：明确告知操作已放弃
                            let what = match purpose {
                                ErrorPurpose::Save => t(Key::BtnSave),
                                ErrorPurpose::SaveAs(_) => t(Key::BtnSaveAs),
                                ErrorPurpose::Open(_) => t(Key::ActionOpen),
                                ErrorPurpose::Startup(_) => t(Key::MainOpenFailedTitle),
                                ErrorPurpose::ExportHtml => t(Key::ActionExportHtml),
                                ErrorPurpose::ExportPdf => t(Key::ActionExportPdf),
                                ErrorPurpose::Print => t(Key::ActionPrint),
                                ErrorPurpose::HistoryDelete(_) => t(Key::HistoryDelete),
                                ErrorPurpose::HistoryClear => t(Key::HistoryClear),
                            };
                            self.ui_state.chrome.notice = tf(Key::NoticeCancelledWhat, &[what]);
                        }
                    }
                }
            }
        }
        self.request_redraw();
    }

    /// 外部变更选择结论：1=重新加载、2=保留当前、3=另存为、4/未知=取消。
    /// 取消不改 pending/hold：弹窗关闭后仍可继续编辑，下次新外部版本才会再提示。
    fn resolve_external(&mut self, id: u16) {
        match id {
            1 => {
                let disk = self.persist_state.persist.pending().map(<[u8]>::to_vec);
                if let (Some(disk), Some(path)) = (disk, self.doc.path.clone()) {
                    // 解码失败时 reload_external 保留 pending/hold，不清用户内容
                    self.reload_external(disk, &path);
                }
            }
            2 => {
                // 保留当前内容：解除自动保存挂起，pending 仍在（标题继续提示外部修改）；
                // 后续自动保存走 save() 冲突分支，先备份再挂起，绝不静默覆盖磁盘。
                self.persist_state.persist.set_hold(false);
                self.ui_state.chrome.notice = t(Key::NoticeExternalKept).into();
            }
            3 => self.save_as(),
            _ => {
                self.ui_state.chrome.notice = t(Key::NoticeExternalDismissed).into();
            }
        }
        self.update_title();
        self.request_redraw();
    }

    /// 未保存确认结论（原有 1/2/3 语义不变）。
    fn resolve_unsaved(&mut self, id: u16, intent: Option<PendingIntent>) {
        let Some(intent) = intent else {
            return;
        };
        match id {
            1 => {
                if self.doc.path.is_some() {
                    self.save_now();
                } else {
                    self.save_as(); // 无路径草稿：保存的唯一出路
                }
                if !self.editor.is_dirty() {
                    self.execute_intent(intent);
                } else {
                    self.ui_state
                        .chrome
                        .notify_error(t(Key::NoticeSaveIncomplete));
                }
            }
            2 => {
                // 明确放弃：先作废恢复快照，避免下次启动复活已放弃内容
                self.clear_recovery();
                self.execute_intent(intent);
            }
            _ => {
                self.ui_state.chrome.notice = t(Key::NoticeCancelDirty).into();
            }
        }
    }

    /// 关窗/退出：脏文档先挂起应用内模态；干净文档直接获准退出（清恢复快照）。
    pub(crate) fn confirm_close(&mut self) {
        let _ = self.begin_unsaved(PendingIntent::Close);
    }

    /// Ctrl+O / 菜单打开：先登记未保存意图，结论出来后再弹文件对话框。
    /// 选择「保存」后才选文件：避免选完文件才发现保存失败。
    pub(crate) fn open_via_dialog(&mut self) {
        let _ = self.begin_unsaved(PendingIntent::Open);
    }

    /// 拖放/最近文件/快速打开：先登记未保存意图，结论出来后再整体替换编辑器。
    pub(crate) fn open_path(&mut self, path: PathBuf) {
        let _ = self.begin_unsaved(PendingIntent::OpenRecent(path));
    }

    /// 未保存处置已完成后的打开对话框（不再确认）。
    fn open_via_dialog_now(&mut self) {
        if let Some(path) = self.ports.dialogs.open_markdown() {
            self.open_path_now(path);
        }
    }

    /// 已完成未保存处置后的打开（不再确认）：加载失败保留当前文档与编辑内容。
    fn open_path_now(&mut self, path: PathBuf) {
        let bytes = match self.ports.store.load(&path) {
            Ok(b) => b,
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeOpenFailed, &[&e]));
                eprintln!("打开失败 {}: {e}", path.display());
                self.show_open_error(&path, &e);
                self.request_redraw();
                return;
            }
        };
        let (text, bom) = match decode(&bytes) {
            Ok(x) => x,
            Err(e) => {
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeOpenFailed, &[&e]));
                eprintln!("打开失败 {}: {e}", path.display());
                self.show_open_error(&path, &e);
                self.request_redraw();
                return;
            }
        };
        self.editor = Editor::new_with_callouts(&text, &self.callouts);
        self.doc.path = Some(path.clone());
        self.doc.history_key = super::history::doc_key(&path);
        self.doc.bom = bom;
        self.doc.baseline = bytes; // 新文件：磁盘内容即基线
        self.persist_state.persist.reset();
        self.persist_state.last_saved_hhmm = None; // 时间属于上一份文档：打开即作废
        self.note_opened(path.clone()); // 登记最近文件（0b）
        self.clear_recovery(); // 旧文档的恢复快照已随打开失效（0b / 3e 修 B）
                               // 新文档：会话与图片基准代次递增，块相关缓存按因果键失配；
                               // 行内公式按 tex 内容寻址，跨文档保留
        self.input_state.revs.session += 1;
        self.input_state.revs.image_base += 1;
        // 换文档：远程图片状态属文档会话，清空以免旧请求/结果串到新文档。
        self.cache.clear_remote();
        // 链接提示属旧文档的命中：换文档立即清除，避免悬停浮层残留旧 URL。
        self.ui_state.hover_link = None;
        self.window_state.scroll_y = 0.0;
        self.window_state.goal_y = 0.0;
        self.window_state.goal_x = None;
        // 表格列宽覆盖按块 id 生效：换文档即整体失效（纯视图态，无副作用）
        self.ui_state.tables.widths.clear();
        self.ui_state.tables.drag = None;
        self.ui_state.outline.scroll = 0.0;
        self.ui_state.outline.hover = None;
        // 新文档：旧查询的命中偏移全部失效（栏保持打开，查询清空）
        self.ui_state.search.clear();
        // 新文档块结构替换：索引必须与编辑器同源重建，后续读取才有效
        self.refresh_index();
        if self.ui_state.outline.open {
            self.refresh_outline();
        }
        // 视口驱动：只准备当前可见块，其余留待滚动/光标路径按需排版
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        // 诊断事件：只记录扩展名与结果，不含完整路径/文件名/正文。
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("none");
        crate::app::record_diag("doc", format!("open ext={ext}"));
        self.ui_state.chrome.notice = tf(Key::NoticeOpened, &[&path_label(Some(path.as_path()))]);
        self.watch_current(); // 失败提示覆盖「已打开」，避免隐藏降级原因
        self.update_title();
        self.set_ime_area();
        self.request_redraw();
    }

    /// Ctrl+N / 菜单新建：先登记未保存意图，结论出来后再以空白草稿替换当前文档。
    pub(crate) fn new_draft(&mut self) {
        let _ = self.begin_unsaved(PendingIntent::New);
    }

    /// 新建的落地点（不再确认）：成功即换新编辑器（新撤销栈、脏标清除）、
    /// 文件监听解除、派生缓存按会话代次失效。
    fn new_draft_now(&mut self) {
        self.editor = Editor::new_with_callouts("", &self.callouts);
        self.doc.path = None;
        self.doc.history_key = super::history::session_key();
        self.doc.bom = false;
        self.doc.baseline = encode("", false); // 空白草稿：基线为空字节
        self.persist_state.persist.reset();
        self.persist_state.last_saved_hhmm = None; // 新草稿未保存过
        self.clear_recovery(); // 旧文档的恢复快照已随新建失效（0b / 3e 修 B）
                               // 新文档：会话与图片基准代次递增，块相关缓存按因果键失配；
                               // 行内公式按 tex 内容寻址，跨文档保留
        self.input_state.revs.session += 1;
        self.input_state.revs.image_base += 1;
        // 新建草稿：远程图片状态属旧文档会话，清空。
        self.cache.clear_remote();
        self.ui_state.hover_link = None;
        self.window_state.scroll_y = 0.0;
        self.window_state.goal_y = 0.0;
        self.window_state.goal_x = None;
        // 表格列宽覆盖按块 id 生效：换文档即整体失效（纯视图态，无副作用）
        self.ui_state.tables.widths.clear();
        self.ui_state.tables.drag = None;
        self.ui_state.outline.scroll = 0.0;
        self.ui_state.outline.hover = None;
        // 新文档：旧查询的命中偏移全部失效（栏保持打开，查询清空）
        self.ui_state.search.clear();
        self.refresh_index();
        if self.ui_state.outline.open {
            self.refresh_outline();
        }
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.ui_state.chrome.notice = t(Key::NoticeNewDraft).into();
        self.watch_current(); // 旧路径解绑：无路径时 watcher.unwatch
        self.update_title();
        self.set_ime_area();
        self.request_redraw();
    }
}

/// 同步填充打印专用 light map；屏幕缓存只读，渲染失败静默留给上层降级。
fn ensure_mermaid_for_print_blocks(
    blocks: Vec<(veil_core::BlockId, String)>,
    screen_dark: bool,
    screen_imgs: &HashMap<veil_core::BlockId, MermaidImage>,
    print_imgs: &mut HashMap<veil_core::BlockId, MermaidImage>,
    renderer: &mut dyn MermaidRenderer,
) {
    // 主题切换会清空屏幕 Mermaid cache，故 light 主题下按 BlockId 命中即为 light 图。
    if !screen_dark {
        for (id, _) in &blocks {
            if let Some(img) = screen_imgs.get(id) {
                print_imgs.entry(*id).or_insert_with(|| img.clone());
            }
        }
    }

    let started = Instant::now();
    let mut rendered = 0usize;
    for (id, source) in blocks {
        // dark 屏幕缓存存在也不复用；但本会话已生成的 light 图可直接命中。
        if print_imgs.contains_key(&id) {
            continue;
        }
        // 与 HTML 导出同一预算及源码上限，保持同步导出语义。
        if rendered >= 16 || started.elapsed() > Duration::from_secs(10) {
            break;
        }
        if source.len() > 128 * 1024 {
            continue;
        }
        rendered += 1;
        let label = format!(
            "mermaid-print:{:016x}",
            crate::app::content_hash(format!("{source}\0false").as_bytes())
        );
        if let Ok(img) = renderer.render_blocking(label, source, false) {
            print_imgs.insert(id, img);
        }
    }
}

/// 崩溃报告数：`data_dir()/logs` 下 `veil-crash-*.txt` 的数量。
/// 只计数，不读取内容；目录不可访问时按 0 处理。
fn count_crash_reports() -> usize {
    let Some(dir) = crate::theme::data_dir().map(|d| d.join("logs")) else {
        return 0;
    };
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return 0;
    };
    read_dir
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("veil-crash-") && name.ends_with(".txt") && entry.path().is_file()
        })
        .count()
}

/// 保存失败的标准出口：重试（主）/ 另存为（次）/ 取消（次）。
fn save_error_buttons() -> Vec<modal::ModalButton> {
    vec![
        modal::ModalButton::new(1, t(Key::BtnRetry), modal::ButtonRole::Primary).with_mnemonic('r'),
        modal::ModalButton::new(2, t(Key::BtnSaveAs), modal::ButtonRole::Secondary),
        modal::ModalButton::new(3, t(Key::BtnCancel), modal::ButtonRole::Secondary),
    ]
}

/// 导出/其它单出口失败：重试（主）/ 取消（次）。
fn retry_cancel_buttons() -> Vec<modal::ModalButton> {
    vec![
        modal::ModalButton::new(1, t(Key::BtnRetry), modal::ButtonRole::Primary).with_mnemonic('r'),
        modal::ModalButton::new(3, t(Key::BtnCancel), modal::ButtonRole::Secondary),
    ]
}

/// 递归收集图片 URL：Markdown `![]()` 与行内 `<img src>` 同一口径。
fn collect_image_urls(nodes: &[InlineNode], session: &veil_core::Session, out: &mut Vec<String>) {
    for n in nodes {
        let url = match &n.kind {
            InlineKind::Image => n.url.and_then(|r| session.text_slice(r)),
            InlineKind::HtmlSelfClosed {
                tag: HtmlTag::Img,
                attrs,
            } => attrs.src.clone(),
            _ => None,
        };
        if let Some(u) = url {
            if is_relative_asset_url(&u) {
                out.push(u);
            }
        }
        collect_image_urls(&n.children, session, out);
    }
}

/// 相对图片 URL：非空、非绝对路径、无 scheme（远程/`data:`/`file:` 不迁移）。
fn is_relative_asset_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.starts_with('/') || u.starts_with('\\') {
        return false;
    }
    if Path::new(u).is_absolute() {
        return false;
    }
    veil_core::link_scheme(u).is_none()
}

/// 冲突副本路径：`<stem>.conflict-<n><ext>`，n 从 1 起取首个不存在者（不覆盖既有副本）。
fn conflict_copy_path(store: &mut dyn FileStore, path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let ext = path.extension().and_then(|e| e.to_str());
    (1..=9999u32).find_map(|n| {
        let name = match ext {
            Some(e) => format!("{stem}.conflict-{n}.{e}"),
            None => format!("{stem}.conflict-{n}"),
        };
        let cand = path.with_file_name(name);
        store.load(&cand).is_err().then_some(cand)
    })
}

#[cfg(test)]
mod print_mermaid_tests {
    use super::ensure_mermaid_for_print_blocks;
    use crate::mermaid_renderer::{MermaidImage, MermaidRenderer};
    use std::collections::{HashMap, VecDeque};
    use veil_core::BlockId;

    struct RecordingMermaid {
        calls: Vec<(String, String, bool)>,
        results: VecDeque<Result<MermaidImage, String>>,
    }

    impl RecordingMermaid {
        fn new(results: Vec<Result<MermaidImage, String>>) -> Self {
            Self {
                calls: Vec::new(),
                results: results.into(),
            }
        }
    }

    impl MermaidRenderer for RecordingMermaid {
        fn request(&mut self, _label: String, _source: String, _dark: bool) {}

        fn poll(&mut self) -> Vec<(String, Result<MermaidImage, String>)> {
            Vec::new()
        }

        fn set_dpi_scale(&mut self, _scale: f32) {}

        fn render_blocking(
            &mut self,
            label: String,
            source: String,
            dark: bool,
        ) -> Result<MermaidImage, String> {
            self.calls.push((label, source, dark));
            self.results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected blocking render".into()))
        }
    }

    fn image(marker: &str) -> MermaidImage {
        MermaidImage {
            data: marker.as_bytes().to_vec(),
            width: 1,
            height: 1,
            svg_text: Some(format!("<svg>{marker}</svg>")),
            px_to_pt: 0.375,
        }
    }

    fn block(id: BlockId) -> Vec<(BlockId, String)> {
        vec![(id, "graph TD\nA-->B".into())]
    }

    fn assert_image(img: &MermaidImage, marker: &str) {
        assert_eq!(img.data, marker.as_bytes());
        assert_eq!(
            img.svg_text.as_deref(),
            Some(format!("<svg>{marker}</svg>").as_str())
        );
    }

    #[test]
    fn pdf_dark_ui_renders_light_even_with_dark_screen_cache() {
        let id = BlockId(7);
        let screen = HashMap::from([(id, image("dark-screen"))]);
        let mut print = HashMap::new();
        let mut renderer = RecordingMermaid::new(vec![Ok(image("light-pdf"))]);

        ensure_mermaid_for_print_blocks(block(id), true, &screen, &mut print, &mut renderer);

        assert_eq!(renderer.calls.len(), 1);
        let (label, source, dark) = &renderer.calls[0];
        assert!(label.starts_with("mermaid-print:"));
        assert_eq!(source, "graph TD\nA-->B");
        assert!(!dark, "PDF 在 dark UI 下也必须请求 light 渲染");
        assert_image(print.get(&id).expect("PDF light 图"), "light-pdf");

        ensure_mermaid_for_print_blocks(block(id), true, &screen, &mut print, &mut renderer);
        assert_eq!(renderer.calls.len(), 1, "打印会话 light map 命中后不重渲");
    }

    #[test]
    fn pdf_dark_ui_preserves_screen_bitmap_and_svg() {
        let id = BlockId(11);
        let dark = image("dark-original");
        let expected_data = dark.data.clone();
        let expected_svg = dark.svg_text.clone();
        let screen = HashMap::from([(id, dark)]);
        let mut print = HashMap::new();
        let mut renderer = RecordingMermaid::new(vec![Ok(image("light-pdf"))]);

        ensure_mermaid_for_print_blocks(block(id), true, &screen, &mut print, &mut renderer);

        let cached = screen.get(&id).expect("dark 屏幕缓存仍应存在");
        assert_eq!(cached.data, expected_data, "屏幕位图字节不得改变");
        assert_eq!(cached.svg_text, expected_svg, "屏幕 SVG 不得改变");
        assert_image(print.get(&id).expect("独立 PDF 图"), "light-pdf");
    }

    #[test]
    fn pdf_light_ui_reuses_light_screen_cache_without_rendering() {
        let id = BlockId(13);
        let screen = HashMap::from([(id, image("light-screen"))]);
        let mut print = HashMap::new();
        let mut renderer = RecordingMermaid::new(Vec::new());

        ensure_mermaid_for_print_blocks(block(id), false, &screen, &mut print, &mut renderer);

        assert!(renderer.calls.is_empty(), "已有 light 图不应重复渲染");
        assert_image(print.get(&id).expect("复用的 light 图"), "light-screen");
    }

    #[test]
    fn pdf_mermaid_failure_keeps_source_fallback_and_can_retry() {
        let id = BlockId(17);
        let screen = HashMap::from([(id, image("dark-screen"))]);
        let mut print = HashMap::new();
        let mut renderer = RecordingMermaid::new(vec![
            Err("injected offline render failure".into()),
            Ok(image("light-retry")),
        ]);

        ensure_mermaid_for_print_blocks(block(id), true, &screen, &mut print, &mut renderer);

        assert!(!print.contains_key(&id), "失败时应留空以走源码围栏降级");
        assert_eq!(renderer.calls.len(), 1);
        assert!(!renderer.calls[0].2);

        ensure_mermaid_for_print_blocks(block(id), true, &screen, &mut print, &mut renderer);

        assert_eq!(renderer.calls.len(), 2, "失败不应让后续重试失效");
        assert!(!renderer.calls[1].2);
        assert_image(print.get(&id).expect("重试成功的 light 图"), "light-retry");
        assert_image(screen.get(&id).expect("屏幕缓存"), "dark-screen");
    }
}
