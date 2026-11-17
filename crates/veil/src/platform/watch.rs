//! 文件系统监听（切片 16）：notify 事件线程 → 非阻塞通道 → 主线程 `poll`。
//!
//! 监听目标文件的**父目录**（非递归）再按文件名过滤：这样 tmp+rename 式原子保存、
//! 外部编辑器替换文件都能感知；直接监听文件会在 rename 后失效。
//! 事件只描述「发生了什么」，是否需要提示/挂起自动保存由 veil-ui 的策略层决定。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use veil_ui::i18n::{t, tf, Key};
use veil_ui::{FileEvent, FileWatcher};

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
            .ok_or_else(|| t(Key::ErrWatchInit).to_string())?;
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|e| {
                tf(
                    Key::ErrWatchPath,
                    &[&e.to_string(), &dir.display().to_string()],
                )
            })?;
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
///
/// 曾评估把「文件名相等」收紧为「父目录也相等」，结论是**不收紧**：
/// 1) 监听只挂载目标文件的父目录（`NonRecursive`），事件本就不会来自其它目录，
///    该分支在正常路径下不存在跨目录误判；
/// 2) 事件路径与 `target` 的目录形态可能不同（相对 vs 绝对、盘符大小写、
///    8.3 短名/符号链接），强行比较父目录会在这些形态差异下漏掉真实变更——
///    漏事件的代价（外部修改不被提示）高于理论上不存在的误判代价。
fn same_target(p: &Path, target: &Path, canonical: Option<&Path>) -> bool {
    p == target
        || canonical.is_some_and(|c| p == c)
        || (p.file_name().is_some() && p.file_name() == target.file_name())
}

#[cfg(test)]
mod tests {
    use super::same_target;
    use std::path::Path;

    /// 父目录非递归监听：同目录下文件名不同的变更必须被过滤掉。
    #[test]
    fn sibling_files_are_not_the_target() {
        let target = Path::new("C:/docs/note.md");
        for other in [
            "C:/docs/other.md",
            "C:/docs/note.md.bak",
            "C:/docs/note.txt",
        ] {
            assert!(
                !same_target(Path::new(other), target, None),
                "同目录其它文件应被忽略：{other}"
            );
        }
    }

    /// 精确路径分支：原样相同即命中。
    #[test]
    fn exact_path_matches() {
        let target = Path::new("C:/docs/note.md");
        assert!(same_target(Path::new("C:/docs/note.md"), target, None));
    }

    /// 规范路径分支：basename 不同（如 8.3 短名/链接别名），只能经 canonical 命中。
    /// 先断言"无 canonical 不命中"，再断言"有 canonical 命中"，确保确实走到该分支
    /// 而不是被"文件名相等"规则代为通过。
    #[test]
    fn canonical_path_hits_even_with_different_basename() {
        let target = Path::new("C:/docs/note.md");
        let canonical = Path::new("C:/real/alias.md");
        assert!(
            !same_target(canonical, target, None),
            "无 canonical 时不应命中（basename 不同，且目录也不同）"
        );
        assert!(
            same_target(canonical, target, Some(canonical)),
            "canonical 分支应命中"
        );
    }

    /// 文件名相等分支：父目录非递归监听，故同名即视为目标（已记录的宽松规则）。
    #[test]
    fn filename_equality_matches_across_directories() {
        let target = Path::new("C:/docs/note.md");
        assert!(same_target(Path::new("C:/other/note.md"), target, None));
    }
}
