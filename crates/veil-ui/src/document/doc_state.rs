//! 文档生命周期：打开/另存/保存（含冲突备份与原子落盘）、自动保存调度、
//! 外部变更轮询与标题派生。纯策略与 IO 编排，绘制与交互不在此文件。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::{App, FileEvent, FileStore};
use crate::docio::{decode, encode};
use crate::editor::Editor;

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
            self.ui_state.chrome.notice = format!("文件监听不可用: {e}");
            eprintln!("文件监听不可用 {}: {e}", path.display());
        }
    }

    /// 外部变更对外呈现的单一状态（状态栏/标题共用）。
    /// 待备份的新版本优先于「已备份」：备份登记只说明旧版本有副本，不能掩盖新版本。
    pub(crate) fn external_state(&self) -> ExternalState {
        self.persist_state.persist.status()
    }

    /// 窗口标题：路径派生。脏标 `*` 前置与冲突提示后缀是 chrome UI 落地前的最小可见载体。
    /// 脏标必须前置：长路径标题在 Windows 标题栏从右侧截断，末尾标记会被吃掉。
    pub(crate) fn title_text(&self) -> String {
        let base = match &self.doc.path {
            Some(p) => format!("veil — {}", p.display()),
            None => "veil".to_string(),
        };
        let mut t = base;
        if self.editor.is_dirty() {
            t.insert_str(0, "* ");
        }
        match self.external_state() {
            ExternalState::None => {}
            ExternalState::Changed => t.push_str(" (外部修改)"),
            ExternalState::BackedUp => t.push_str(" (外部修改已备份)"),
            ExternalState::Missing => t.push_str(" (文件缺失)"),
        }
        t
    }

    pub(crate) fn update_title(&self) {
        if let Some(w) = &self.window_state.window {
            w.set_title(&self.title_text());
        }
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
                self.ui_state.chrome.notice = "草稿无关联文件；可用 Ctrl+Shift+S 另存为".into();
                eprintln!("草稿无关联文件，保存被忽略");
                self.request_redraw();
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
                        self.ui_state.chrome.notice = format!("外部版本备份失败，取消保存: {e}");
                        eprintln!("外部修改备份失败，取消保存（不覆盖原文件）: {e}");
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
        match self.ports.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                self.doc.baseline = data;
                self.persist_state.persist.keep_notice_only(); // 备份登记作废，「已备份」提示保留
                self.persist_state.persist.clear_pending(); // 磁盘已是本版内容：待备份提示作废
                self.persist_state.persist.set_missing(false); // 删除/不可读状态由本次写入恢复
                if external.is_none() {
                    self.persist_state.persist.clear_backup(); // 本次无冲突（含手工覆盖后）→ 清提示
                }
                if kind == SaveKind::Manual {
                    self.persist_state.persist.set_hold(false);
                    self.ui_state.chrome.notice = format!("已保存 {}", path.display());
                }
                self.persist_state.persist.clear_activity(); // 已干净：取消自动保存调度
                self.clear_recovery(); // 磁盘已是最新：恢复副本作废（0b）
                self.update_title();
            }
            Err(e) => {
                self.ui_state.chrome.notice = format!("保存失败: {e}");
                eprintln!("保存失败 {}: {e}", path.display());
            }
        }
        self.request_redraw();
    }

    /// 把磁盘现存版本写到同目录冲突副本（首个未占用序号，经 FileStore 原子写）。
    pub(crate) fn write_conflict_copy(
        &mut self,
        path: &Path,
        disk: &[u8],
    ) -> Result<PathBuf, String> {
        let bp = conflict_copy_path(&mut *self.ports.store, path)
            .ok_or_else(|| "冲突副本名已耗尽".to_string())?;
        self.ports.store.save(&bp, disk)?;
        Ok(bp)
    }

    /// 自动保存判定：距上次内容编辑/自动保存尝试达静默期，且无路径/IME/冲突挂起阻碍。
    pub(crate) fn autosave_due(&self, now: Instant) -> bool {
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
        self.persist_state.persist.note_activity(now);
        self.save(SaveKind::Auto);
        self.request_redraw();
    }

    /// 恢复写盘判定（0b）：脏且非 IME 组合期即候选（无路径草稿/冲突挂起也覆盖）；
    /// 正常路径按静默期（复用自动保存的 2s），上次失败后按退避节奏重试。
    pub(crate) fn recovery_write_due(&self, now: Instant) -> bool {
        if !self.editor.is_dirty() || self.editor.preedit().is_some() {
            return false;
        }
        if let Some(t) = self.persist_state.recovery_failed_at {
            return now.saturating_duration_since(t) >= RECOVERY_RETRY;
        }
        self.persist_state
            .autosave
            .deadline(self.persist_state.persist.activity())
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
            return; // 磁盘已是当前内容快照
        }
        match self.ports.recovery.save(&data) {
            Ok(()) => {
                self.persist_state.recovery_hash = Some(hash);
                self.persist_state.recovery_failed_at = None;
            }
            Err(e) => {
                self.persist_state.recovery_failed_at = Some(now);
                self.ui_state.chrome.notice = format!("崩溃恢复写入失败: {e}");
                eprintln!("崩溃恢复写入失败: {e}");
            }
        }
    }

    /// 保存/另存为成功后清除恢复副本（0b）。清除失败仅记录：残留文件在下次启动时
    /// 经「恢复内容 == 磁盘内容」判定自清，不会造成错误恢复。
    pub(crate) fn clear_recovery(&mut self) {
        if self.persist_state.recovery_hash.is_none() {
            return; // 本会话未写过恢复文件，磁盘上不会有属于本内容的副本
        }
        if let Err(e) = self.ports.recovery.clear() {
            eprintln!("恢复副本清理失败（下次启动将自清）: {e}");
        }
        self.persist_state.recovery_hash = None;
        self.persist_state.recovery_failed_at = None;
    }

    /// 打开/另存为成功时登记最近文件并持久化（0b）。写失败不阻断主流程。
    pub(crate) fn note_opened(&mut self, path: PathBuf) {
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
                        self.ui_state.chrome.notice = format!("文件已恢复：{}", path.display());
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
                self.persist_state.persist.begin_external(disk); // 脏文档绝不静默覆盖外部版本
                self.ui_state.chrome.notice = "检测到外部修改：保存时将先备份磁盘版本".into();
                self.update_title();
                self.request_redraw();
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
                    "文件已被外部删除；保存将重建".into()
                } else {
                    "文件暂不可读（可能被外部程序占用）".into()
                };
                self.update_title();
                self.request_redraw();
            }
        }
    }

    /// Ctrl+E：经注入的 Exporter 导出 HTML。草稿无路径 / 导出失败走 stderr 不崩溃
    /// （同 save_now 草稿分支语义；界面呈现待 chrome UI）。导出不清脏标（源文件未变）。
    pub(crate) fn export_now(&mut self) {
        match self
            .ports
            .exporter
            .export_html(self.editor.session(), self.doc.path.as_deref())
        {
            Ok(p) => {
                self.ui_state.chrome.notice = format!("已导出 {}", p.display());
                eprintln!("已导出 {}", p.display());
            }
            Err(e) => {
                self.ui_state.chrome.notice = format!("导出失败: {e}");
                eprintln!("导出失败: {e}");
            }
        }
        self.request_redraw();
    }

    /// Ctrl+Shift+S：另存为。写入成功才切换关联路径/基线并清脏；失败保留旧路径与脏标。
    /// 新路径改变图片相对基准：图片代次递增，图片/布局/表格按因果键失配重取。
    pub(crate) fn save_as(&mut self) {
        let suggested = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("未命名.md")
            .to_string();
        let Some(path) = self.ports.dialogs.save_markdown(&suggested) else {
            self.ui_state.chrome.notice = "另存为已取消".into();
            self.request_redraw();
            return;
        };
        let data = encode(&self.editor.session().serialize(), self.doc.bom);
        match self.ports.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                self.doc.path = Some(path.clone());
                self.doc.baseline = data;
                self.persist_state.persist.reset();
                self.input_state.revs.image_base += 1; // 图片相对基准变化：相关派生缓存键失配
                self.clear_recovery(); // 旧路径的恢复副本已随另存失效（0b）
                self.note_opened(path.clone()); // 登记最近文件（0b）
                self.ui_state.chrome.notice = format!("已另存为 {}", path.display());
                self.watch_current(); // 监听目标随关联路径切换；失败提示覆盖成功提示
                self.update_title();
            }
            Err(e) => {
                self.ui_state.chrome.notice = format!("另存为失败: {e}");
                eprintln!("另存为失败 {}: {e}", path.display());
            }
        }
        self.request_redraw();
    }

    /// 未保存内容的统一处置（3e 修 B）：脏文档弹三选一，返回「是否继续」。
    /// - 保存：有路径走保存，无路径草稿必须另存为；未完成（失败或取消）→ 取消继续；
    /// - 放弃：立即作废恢复快照，避免下次启动复活已放弃的内容；
    /// - 取消：保持编辑状态（状态栏提示，不静默）。
    pub(crate) fn resolve_unsaved(&mut self) -> bool {
        if !self.editor.is_dirty() {
            return true; // 干净文档：无需确认
        }
        let path = self.doc.path.clone();
        match self.ports.confirmer.confirm_unsaved(path.as_deref()) {
            crate::app::UnsavedChoice::Cancel => {
                self.ui_state.chrome.notice = "已取消：当前文档仍有未保存的修改".into();
                self.request_redraw();
                false
            }
            crate::app::UnsavedChoice::Discard => {
                self.clear_recovery();
                true
            }
            crate::app::UnsavedChoice::Save => {
                if self.doc.path.is_some() {
                    self.save_now();
                } else {
                    self.save_as(); // 无路径草稿：保存的唯一出路
                }
                if !self.editor.is_dirty() {
                    return true;
                }
                self.ui_state.chrome.notice = "保存未完成（失败或取消），操作已取消".into();
                self.request_redraw();
                false
            }
        }
    }

    /// 关窗/退出：脏文档先三选一；确认退出后清恢复快照（干净退出不留残稿）。
    /// 返回 false = 留在编辑器继续（用户取消）；仅当返回 true 才允许事件循环退出。
    pub(crate) fn confirm_close(&mut self) -> bool {
        if !self.resolve_unsaved() {
            return false;
        }
        self.clear_recovery();
        true
    }

    /// Ctrl+O / 菜单打开：先处置未保存内容（保存/放弃/取消），再弹文件对话框。
    /// 选择「保存」后才选文件：避免选完文件才发现保存失败。
    pub(crate) fn open_via_dialog(&mut self) {
        if !self.resolve_unsaved() {
            return;
        }
        if let Some(path) = self.ports.dialogs.open_markdown() {
            self.open_path_now(path);
        }
    }

    /// 拖放/最近文件/快速打开：先处置未保存内容，再整体替换编辑器。
    pub(crate) fn open_path(&mut self, path: PathBuf) {
        if !self.resolve_unsaved() {
            return;
        }
        self.open_path_now(path);
    }

    /// 已完成未保存处置后的打开（不再确认）：加载失败保留当前文档与编辑内容。
    fn open_path_now(&mut self, path: PathBuf) {
        let bytes = match self.ports.store.load(&path) {
            Ok(b) => b,
            Err(e) => {
                self.ui_state.chrome.notice = format!("打开失败: {e}");
                eprintln!("打开失败 {}: {e}", path.display());
                self.request_redraw();
                return;
            }
        };
        let (text, bom) = match decode(&bytes) {
            Ok(x) => x,
            Err(e) => {
                self.ui_state.chrome.notice = format!("打开失败: {e}");
                eprintln!("打开失败 {}: {e}", path.display());
                self.request_redraw();
                return;
            }
        };
        self.editor = Editor::new(&text);
        self.doc.path = Some(path.clone());
        self.doc.bom = bom;
        self.doc.baseline = bytes; // 新文件：磁盘内容即基线
        self.persist_state.persist.reset();
        self.note_opened(path.clone()); // 登记最近文件（0b）
        self.clear_recovery(); // 旧文档的恢复快照已随打开失效（0b / 3e 修 B）
        // 新文档：会话与图片基准代次递增，块相关缓存按因果键失配；
        // 行内公式按 tex 内容寻址，跨文档保留
        self.input_state.revs.session += 1;
        self.input_state.revs.image_base += 1;
        self.window_state.scroll_y = 0.0;
        self.window_state.goal_x = None;
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
        self.ui_state.chrome.notice = format!("已打开 {}", path.display());
        self.watch_current(); // 失败提示覆盖「已打开」，避免隐藏降级原因
        self.update_title();
        self.set_ime_area();
        self.request_redraw();
    }

    /// Ctrl+N / 菜单新建：先处置未保存内容，再以空白草稿替换当前文档。
    /// 成功即换新编辑器（新撤销栈、脏标清除）、文件监听解除、派生缓存按会话代次失效。
    pub(crate) fn new_draft(&mut self) {
        if !self.resolve_unsaved() {
            return;
        }
        self.editor = Editor::new("");
        self.doc.path = None;
        self.doc.bom = false;
        self.doc.baseline = encode("", false); // 空白草稿：基线为空字节
        self.persist_state.persist.reset();
        self.clear_recovery(); // 旧文档的恢复快照已随新建失效（0b / 3e 修 B）
        // 新文档：会话与图片基准代次递增，块相关缓存按因果键失配；
        // 行内公式按 tex 内容寻址，跨文档保留
        self.input_state.revs.session += 1;
        self.input_state.revs.image_base += 1;
        self.window_state.scroll_y = 0.0;
        self.window_state.goal_x = None;
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
        self.ui_state.chrome.notice = "已新建空白文档".into();
        self.watch_current(); // 旧路径解绑：无路径时 watcher.unwatch
        self.update_title();
        self.set_ime_area();
        self.request_redraw();
    }
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
