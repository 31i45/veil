//! 文档生命周期：打开/另存/保存（含冲突备份与原子落盘）、自动保存调度、
//! 外部变更轮询与标题派生。纯策略与 IO 编排，绘制与交互不在此文件。

use super::*;
use crate::chrome::ExternalState;
use crate::docio::decode;

/// 保存来源：自动保存绝不覆盖外部修改，手工保存经备份后允许覆盖。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SaveKind {
    Manual,
    Auto,
}

impl App {
    /// 把监听目标切换到当前路径（草稿则停止监听）。失败仅提示，不影响编辑。
    pub(super) fn watch_current(&mut self) {
        let Some(path) = self.path.clone() else {
            self.watcher.unwatch();
            return;
        };
        if let Err(e) = self.watcher.watch(&path) {
            self.chrome.notice = format!("文件监听不可用: {e}");
            eprintln!("文件监听不可用 {}: {e}", path.display());
        }
    }

    /// 外部变更对外呈现的单一状态（状态栏/标题共用）。
    /// 待备份的新版本优先于「已备份」：备份登记只说明旧版本有副本，不能掩盖新版本。
    pub(super) fn external_state(&self) -> ExternalState {
        if self.external_changed.is_some() {
            ExternalState::Changed
        } else if self.conflict {
            ExternalState::BackedUp
        } else if self.external_missing {
            ExternalState::Missing
        } else {
            ExternalState::None
        }
    }

    /// 窗口标题：路径派生。脏标 ` *` 与冲突提示后缀是 chrome UI 落地前的最小可见载体。
    pub(super) fn title_text(&self) -> String {
        let base = match &self.path {
            Some(p) => format!("veil — {}", p.display()),
            None => "veil".to_string(),
        };
        let mut t = base;
        if self.editor.is_dirty() {
            t.push_str(" *");
        }
        match self.external_state() {
            ExternalState::None => {}
            ExternalState::Changed => t.push_str(" (外部修改)"),
            ExternalState::BackedUp => t.push_str(" (外部修改已备份)"),
            ExternalState::Missing => t.push_str(" (文件缺失)"),
        }
        t
    }

    pub(super) fn update_title(&self) {
        if let Some(w) = &self.window {
            w.set_title(&self.title_text());
        }
    }

    /// Ctrl+S：显式保存（允许在备份外部版本后覆盖）。
    pub(super) fn save_now(&mut self) {
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
    pub(super) fn save(&mut self, kind: SaveKind) {
        let Some(path) = self.path.clone() else {
            if kind == SaveKind::Manual {
                self.chrome.notice = "草稿无关联文件；可用 Ctrl+Shift+S 另存为".into();
                eprintln!("草稿无关联文件，保存被忽略");
                self.request_redraw();
            }
            return;
        };
        let data = encode(&self.editor.session().serialize(), self.bom);
        let external = match self.store.load(&path) {
            Ok(disk) if disk != self.baseline => Some(disk),
            _ => None,
        };
        if let Some(disk) = &external {
            // 同一外部版本只备份一次（自动冲突已写副本 → 手工保存不再重复）
            if self.external_saved.as_deref() != Some(disk.as_slice()) {
                match self.write_conflict_copy(&path, disk) {
                    Ok(bp) => {
                        self.conflict = true;
                        self.external_saved = Some(disk.clone());
                        eprintln!("检测到外部修改，原文件已备份到 {}", bp.display());
                    }
                    Err(e) => {
                        self.chrome.notice = format!("外部版本备份失败，取消保存: {e}");
                        eprintln!("外部修改备份失败，取消保存（不覆盖原文件）: {e}");
                        self.update_title();
                        self.request_redraw();
                        return;
                    }
                }
            } else {
                self.conflict = true;
            }
            if kind == SaveKind::Auto {
                self.autosave_hold = true;
                eprintln!("自动保存已暂停：{} 被外部修改", path.display());
                self.update_title();
                return;
            }
        }
        match self.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                self.baseline = data;
                self.external_saved = None; // 基线已与磁盘一致，备份登记作废
                self.external_changed = None; // 磁盘已是本版内容：待备份提示作废
                self.external_missing = false; // 删除/不可读状态由本次写入恢复
                if external.is_none() {
                    self.conflict = false; // 本次无冲突（含手工覆盖后）→ 清提示
                }
                if kind == SaveKind::Manual {
                    self.autosave_hold = false;
                    self.chrome.notice = format!("已保存 {}", path.display());
                }
                self.last_activity = None; // 已干净：取消自动保存调度
                self.update_title();
            }
            Err(e) => {
                self.chrome.notice = format!("保存失败: {e}");
                eprintln!("保存失败 {}: {e}", path.display());
            }
        }
        self.request_redraw();
    }

    /// 把磁盘现存版本写到同目录冲突副本（首个未占用序号，经 FileStore 原子写）。
    pub(super) fn write_conflict_copy(
        &mut self,
        path: &Path,
        disk: &[u8],
    ) -> Result<PathBuf, String> {
        let bp = conflict_copy_path(&mut *self.store, path)
            .ok_or_else(|| "冲突副本名已耗尽".to_string())?;
        self.store.save(&bp, disk)?;
        Ok(bp)
    }

    /// 自动保存判定：距上次内容编辑/自动保存尝试达静默期，且无路径/IME/冲突挂起阻碍。
    pub(super) fn autosave_due(&self, now: Instant) -> bool {
        let Some(last) = self.last_activity else {
            return false;
        };
        self.autosave.due(
            self.editor.is_dirty(),
            self.path.is_some(),
            self.editor.preedit().is_some(),
            self.autosave_hold,
            now.saturating_duration_since(last),
        )
    }

    /// 尝试自动保存（事件循环空闲回调调用）。无论成败都推进计时起点，避免忙轮询。
    pub(super) fn maybe_autosave(&mut self, now: Instant) {
        if !self.autosave_due(now) {
            return;
        }
        self.last_activity = Some(now);
        self.save(SaveKind::Auto);
        self.request_redraw();
    }

    /// 外部变更轮询（空闲回调调用）：只置状态与提示，绝不自动改文档内容或落盘。
    ///
    /// 检测：监听事件 → 读磁盘 → 与基线/已备份版本比较。备份仍只在保存覆盖前执行，
    /// 避免外部程序每保存一次就在文档旁积一份副本；同一版本/已备份版本不重复提示。
    pub(super) fn poll_external(&mut self) {
        let events = self.watcher.poll();
        if events.is_empty() {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        let removed = events.contains(&FileEvent::Removed);
        match self.store.load(&path) {
            Ok(disk) => {
                let recovered = std::mem::take(&mut self.external_missing);
                if disk == self.baseline {
                    // 外部内容回到基线：撤销「待备份」状态（已备份提示保留至下次保存）
                    if self.external_changed.take().is_some() && !self.conflict {
                        self.autosave_hold = false;
                    }
                    if recovered {
                        self.chrome.notice = format!("文件已恢复：{}", path.display());
                        self.update_title();
                        self.request_redraw();
                    }
                    return;
                }
                if self.external_saved.as_deref() == Some(disk.as_slice())
                    || self.external_changed.as_deref() == Some(disk.as_slice())
                {
                    return; // 已备份 / 同一外部版本：不重复提示
                }
                self.external_changed = Some(disk);
                self.autosave_hold = true; // 脏文档绝不静默覆盖外部版本
                self.chrome.notice = "检测到外部修改：保存时将先备份磁盘版本".into();
                self.update_title();
                self.request_redraw();
            }
            Err(_) => {
                if self.external_missing {
                    return;
                }
                self.external_missing = true;
                self.external_changed = None;
                if !self.conflict {
                    self.autosave_hold = false; // 重建文件不构成覆盖，允许自动保存
                }
                self.chrome.notice = if removed {
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
    pub(super) fn export_now(&mut self) {
        match self
            .exporter
            .export_html(self.editor.session(), self.path.as_deref())
        {
            Ok(p) => {
                self.chrome.notice = format!("已导出 {}", p.display());
                eprintln!("已导出 {}", p.display());
            }
            Err(e) => {
                self.chrome.notice = format!("导出失败: {e}");
                eprintln!("导出失败: {e}");
            }
        }
        self.request_redraw();
    }

    /// Ctrl+Shift+S：另存为。写入成功才切换关联路径/基线并清脏；失败保留旧路径与脏标。
    /// 新路径改变图片相对基准：清图片缓存与布局，按新位置重新解析。
    pub(super) fn save_as(&mut self) {
        let suggested = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("未命名.md")
            .to_string();
        let Some(path) = self.dialogs.save_markdown(&suggested) else {
            self.chrome.notice = "另存为已取消".into();
            self.request_redraw();
            return;
        };
        let data = encode(&self.editor.session().serialize(), self.bom);
        match self.store.save(&path, &data) {
            Ok(()) => {
                self.editor.mark_saved();
                self.path = Some(path.clone());
                self.baseline = data;
                self.conflict = false;
                self.external_saved = None;
                self.external_changed = None;
                self.external_missing = false;
                self.autosave_hold = false;
                self.last_activity = None;
                self.inline_images.clear();
                self.image_failed.clear();
                self.layouts.clear();
                self.rasters.clear();
                self.tables.clear();
                self.chrome.notice = format!("已另存为 {}", path.display());
                self.watch_current(); // 监听目标随关联路径切换；失败提示覆盖成功提示
                self.update_title();
            }
            Err(e) => {
                self.chrome.notice = format!("另存为失败: {e}");
                eprintln!("另存为失败 {}: {e}", path.display());
            }
        }
        self.request_redraw();
    }

    /// Ctrl+O / 菜单打开：脏文档先拒绝（v0 无确认对话框，宁丢操作不丢数据），
    /// 拒绝时不弹对话框（避免选完文件才发现无法打开）。
    pub(super) fn open_via_dialog(&mut self) {
        if self.editor.is_dirty() {
            self.chrome.notice = "当前文档未保存，已取消打开".into();
            self.request_redraw();
            return;
        }
        if let Some(path) = self.dialogs.open_markdown() {
            self.open_path(path);
        }
    }

    /// 拖放打开：脏文档拒绝替换（v0 无确认对话框，宁丢操作不丢数据）；
    /// 成功后整体替换编辑器（新文档新撤销栈）、清全部缓存、复位滚动。
    pub(super) fn open_path(&mut self, path: PathBuf) {
        if self.editor.is_dirty() {
            self.chrome.notice = "当前文档未保存，已拒绝打开".into();
            eprintln!("当前文档未保存，拒绝打开 {}", path.display());
            self.request_redraw();
            return;
        }
        let bytes = match self.store.load(&path) {
            Ok(b) => b,
            Err(e) => {
                self.chrome.notice = format!("打开失败: {e}");
                eprintln!("打开失败 {}: {e}", path.display());
                self.request_redraw();
                return;
            }
        };
        let (text, bom) = match decode(&bytes) {
            Ok(x) => x,
            Err(e) => {
                self.chrome.notice = format!("打开失败: {e}");
                eprintln!("打开失败 {}: {e}", path.display());
                self.request_redraw();
                return;
            }
        };
        self.editor = Editor::new(&text);
        self.path = Some(path.clone());
        self.bom = bom;
        self.baseline = bytes; // 新文件：磁盘内容即基线
        self.conflict = false;
        self.external_saved = None;
        self.external_changed = None;
        self.external_missing = false;
        self.autosave_hold = false;
        self.last_activity = None;
        self.layouts.clear();
        self.rasters.clear();
        self.math_imgs.clear();
        self.math_failed.clear();
        self.inline_images.clear(); // 新文档：相对路径基准变化，失败记忆一并作废
        self.image_failed.clear();
        self.tables.clear();
        self.scroll_y = 0.0;
        self.goal_x = None;
        // 新文档块结构替换：索引必须与编辑器同源重建，后续读取才有效
        self.refresh_index();
        // 视口驱动：只准备当前可见块，其余留待滚动/光标路径按需排版
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.chrome.notice = format!("已打开 {}", path.display());
        self.watch_current(); // 失败提示覆盖「已打开」，避免隐藏降级原因
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
