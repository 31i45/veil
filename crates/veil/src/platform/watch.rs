//! 文件系统监听（切片 16）：notify 事件线程 → 非阻塞通道 → 主线程 `poll`。
//!
//! 监听目标文件的**父目录**（非递归）再按文件名过滤：这样 tmp+rename 式原子保存、
//! 外部编辑器替换文件都能感知；直接监听文件会在 rename 后失效。
//! 事件只描述「发生了什么」，是否需要提示/挂起自动保存由 veil-egui 的策略层决定。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use veil_egui::{FileEvent, FileWatcher};

pub struct NotifyWatcher {
    watcher: Option<RecommendedWatcher>,
    target: Option<PathBuf>,
    /// 目标路径的规范形式（存在时）；用于过滤 notify 给出的绝对路径。
    canonical: Option<PathBuf>,
    rx: Receiver<notify::Result<Event>>,
}

impl NotifyWatcher {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res); // 接收端在 unwatch/析构后丢弃，不 panic
            },
            Config::default(),
        )
        .ok();
        Self {
            watcher,
            target: None,
            canonical: None,
            rx,
        }
    }
}

impl Default for NotifyWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWatcher for NotifyWatcher {
    fn watch(&mut self, path: &Path) -> Result<(), String> {
        self.unwatch();
        let watcher = self
            .watcher
            .as_mut()
            .ok_or_else(|| "文件监听初始化失败".to_string())?;
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("{e}（监听 {}）", dir.display()))?;
        self.canonical = path.canonicalize().ok();
        self.target = Some(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self) {
        let Some(target) = self.target.take() else {
            return;
        };
        self.canonical = None;
        if let Some(w) = self.watcher.as_mut() {
            let dir = match target.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };
            let _ = w.unwatch(&dir);
        }
        // 丢弃旧路径的残留事件，避免切换文档后误判
        while self.rx.try_recv().is_ok() {}
    }

    fn poll(&mut self) -> Vec<FileEvent> {
        let Some(target) = self.target.clone() else {
            return Vec::new();
        };
        let mut removed = false;
        let mut changed = false;
        while let Ok(res) = self.rx.try_recv() {
            // notify 的错误事件（如系统缓冲区溢出）不改变「哪一版内容在磁盘上」，
            // 跳过但继续消费，避免一次错误吞掉同批次的真实事件。
            let Ok(ev) = res else {
                continue;
            };
            if !ev
                .paths
                .iter()
                .any(|p| same_target(p, &target, self.canonical.as_deref()))
            {
                continue;
            }
            match ev.kind {
                EventKind::Access(_) => {} // 纯读取/打开不改变内容
                EventKind::Remove(_) => removed = true,
                EventKind::Modify(ModifyKind::Name(RenameMode::From)) => removed = true,
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                    // 原子替换：paths = [旧名, 新名]；旧名是目标 → 目标被移走
                    if ev
                        .paths
                        .first()
                        .is_some_and(|p| same_target(p, &target, self.canonical.as_deref()))
                    {
                        removed = true;
                    } else {
                        changed = true;
                    }
                }
                _ => changed = true,
            }
        }
        // 同批次只给一个结论：删除优先（读盘失败会走「缺失」分支）
        if removed {
            vec![FileEvent::Removed]
        } else if changed {
            vec![FileEvent::Changed]
        } else {
            Vec::new()
        }
    }
}

/// 事件路径是否指向监听目标：只监听父目录非递归，故文件名相等即同一文件；
/// 规范路径/原样路径再做一次精确匹配以覆盖相对路径差异。
fn same_target(p: &Path, target: &Path, canonical: Option<&Path>) -> bool {
    p == target
        || canonical.is_some_and(|c| p == c)
        || (p.file_name().is_some() && p.file_name() == target.file_name())
}
