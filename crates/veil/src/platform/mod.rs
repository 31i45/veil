//! 平台能力注入（产品层）：系统对话框、文件监听、主题配置/系统外观适配为 veil-egui 的 trait。
//! 对话框与系统外观：Windows 手写 Win32 FFI（零新增依赖），其他平台返回兜底实现。
//! 文件监听：notify 事件线程（切片 16），跨平台同一实现。配置读写：std 原子写（切片 17）。

#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod windows;
mod watch;

use std::path::PathBuf;

use veil_egui::{
    ConfigStore, FileDialog, FileWatcher, SystemTheme, ThemeEnv, WindowEffects,
};
#[cfg(not(windows))]
use veil_egui::NativeWindow;

/// 系统文件对话框。非 Windows 平台为「总是取消」的兜底实现。
pub fn dialogs() -> Box<dyn FileDialog> {
    #[cfg(windows)]
    {
        Box::new(windows::WinDialogs)
    }
    #[cfg(not(windows))]
    {
        Box::new(NoDialogs)
    }
}

/// 文件系统监听：notify 建议实现（事件线程 + 非阻塞通道）。
/// 初始化失败时返回的对象仍可用：`watch` 返回 Err，由视图层降级为提示。
pub fn watcher() -> Box<dyn FileWatcher> {
    Box::new(watch::NotifyWatcher::new())
}

#[cfg(not(windows))]
struct NoDialogs;
#[cfg(not(windows))]
impl FileDialog for NoDialogs {
    fn open_markdown(&mut self) -> Option<std::path::PathBuf> {
        None
    }
    fn save_markdown(&mut self, _suggested: &str) -> Option<std::path::PathBuf> {
        None
    }
}

/// 主题配置读写：`%APPDATA%\Veil\config.toml`，原子写（同目录 tmp + rename）并按需建目录。
/// 文件不存在属正常状态（首次运行），`load` 返回 Err 由视图层静默使用默认配置。
pub struct FsConfig {
    pub path: Option<PathBuf>,
}

impl ConfigStore for FsConfig {
    fn load(&mut self) -> Result<String, String> {
        let path = self.path.as_ref().ok_or("无法定位配置目录")?;
        let bytes =
            std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // 非法 UTF-8 不整文件拒绝：逐行解析时非法值会被忽略并提示（容错优于失效）
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn save(&mut self, text: &str) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("无法定位配置目录")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("{e}（创建 {}）", dir.display()))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".veil-tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| format!("{e}（写入 {}）", path.display()))
    }
}

/// 主题能力装配：配置路径 + 系统外观 + 窗口外观（非 Windows 均为安全兜底）。
pub fn theme_env() -> ThemeEnv {
    ThemeEnv {
        config: Box::new(FsConfig {
            path: veil_egui::config_path(),
        }),
        system: system_theme(),
        effects: window_effects(),
    }
}

#[cfg(windows)]
fn system_theme() -> Box<dyn SystemTheme> {
    Box::new(theme::WinSystemTheme)
}
#[cfg(not(windows))]
fn system_theme() -> Box<dyn SystemTheme> {
    Box::new(NoSystemTheme)
}

#[cfg(windows)]
fn window_effects() -> Box<dyn WindowEffects> {
    Box::new(theme::WinEffects)
}
#[cfg(not(windows))]
fn window_effects() -> Box<dyn WindowEffects> {
    Box::new(NoEffects)
}

#[cfg(not(windows))]
struct NoSystemTheme;
#[cfg(not(windows))]
impl SystemTheme for NoSystemTheme {
    fn is_dark(&mut self) -> Option<bool> {
        None
    }
}

#[cfg(not(windows))]
struct NoEffects;
#[cfg(not(windows))]
impl WindowEffects for NoEffects {
    fn apply(&mut self, _window: NativeWindow, _dark: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_writes_atomically_and_creates_directory() {
        let dir = std::env::temp_dir().join(format!(
            "veil-config-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("Veil").join("config.toml");
        let mut c = FsConfig {
            path: Some(path.clone()),
        };
        assert!(c.load().is_err(), "首次运行无配置文件");
        c.save("[theme]\nmode = \"dark\"\n").expect("原子写");
        assert_eq!(c.load().unwrap(), "[theme]\nmode = \"dark\"\n");
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".veil-tmp");
        assert!(!PathBuf::from(tmp).exists(), "临时文件须被 rename 消费");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
