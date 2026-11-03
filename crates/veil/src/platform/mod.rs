//! 平台能力注入（产品层）：系统对话框、文件监听、主题配置/系统外观适配为 veil-ui 的 trait。
//! 对话框与系统外观：Windows 手写 Win32 FFI（零新增依赖），其他平台返回兜底实现。
//! 文件监听：notify 事件线程（切片 16），跨平台同一实现。配置读写：std 原子写（切片 17）。

#[cfg(windows)]
mod confirm;
#[cfg(windows)]
mod print;
#[cfg(windows)]
mod theme;
mod watch;
#[cfg(windows)]
mod wic;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;

#[cfg(not(windows))]
use veil_ui::NativeWindow;
use veil_ui::{
    ConfigStore, FileDialog, FileWatcher, ImageData, PersistEnv, RecoveryStore, SessionStore,
    SystemTheme, ThemeEnv, WindowEffects,
};

/// 非 PNG 图片解码（阶段 1）：Windows 走 WIC（系统编解码器，覆盖 JPEG/WebP/BMP/GIF/TIFF）；
/// 其他平台返回可读错误，PNG 路径仍由产品层 png 解码器承担。
#[cfg(windows)]
pub fn decode_image(path: &std::path::Path) -> Result<ImageData, String> {
    wic::decode_image(path)
}

#[cfg(not(windows))]
pub fn decode_image(_path: &std::path::Path) -> Result<ImageData, String> {
    Err("当前平台仅支持 PNG 图片".into())
}

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

/// 打印能力装配（3b）：Windows 手写 Win32 GDI 打印；其他平台为「提示不支持」兜底。
pub fn print() -> Box<dyn veil_ui::Print> {
    #[cfg(windows)]
    {
        Box::new(print::WinPrint)
    }
    #[cfg(not(windows))]
    {
        Box::new(veil_ui::NoPrint)
    }
}

/// 未保存内容确认（3e 修 B）：Windows 原生 MessageBoxW 三选一；
/// 其他平台为「取消」兜底（宁可拒绝退出，也绝不静默丢内容）。
pub fn confirmer() -> Box<dyn veil_ui::Confirmer> {
    #[cfg(windows)]
    {
        Box::new(confirm::WinConfirm)
    }
    #[cfg(not(windows))]
    {
        Box::new(CancelAll)
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

#[cfg(not(windows))]
struct CancelAll;
#[cfg(not(windows))]
impl veil_ui::Confirmer for CancelAll {
    fn confirm_unsaved(&mut self, _file: Option<&std::path::Path>) -> veil_ui::UnsavedChoice {
        veil_ui::UnsavedChoice::Cancel
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
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // 非法 UTF-8 不整文件拒绝：逐行解析时非法值会被忽略并提示（容错优于失效）
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn save(&mut self, text: &str) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("无法定位配置目录")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{e}（创建 {}）", dir.display()))?;
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
            path: veil_ui::config_path(),
        }),
        system: system_theme(),
        effects: window_effects(),
    }
}

/// 原子写小助手：同目录 tmp + rename，并按需创建父目录（config/recovery/session 共用）。
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{e}（创建 {}）", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".veil-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{e}（写入 {}）", path.display()))
}

/// 崩溃恢复草稿（0b）：`<数据目录>/recovery.dat` 单槽，原子写；清除时文件不存在视为成功。
pub struct FsRecovery {
    pub path: Option<PathBuf>,
}

impl RecoveryStore for FsRecovery {
    fn load(&mut self) -> Result<Vec<u8>, String> {
        let path = self.path.as_ref().ok_or("无法定位数据目录")?;
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }
    fn save(&mut self, data: &[u8]) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("无法定位数据目录")?;
        atomic_write(path, data)
    }
    fn clear(&mut self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("无法定位数据目录")?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

/// 会话状态（0b 最近文件）：`<数据目录>/session.txt` 小文本，原子写。
pub struct FsSession {
    pub path: Option<PathBuf>,
}

impl SessionStore for FsSession {
    fn load(&mut self) -> Result<String, String> {
        let path = self.path.as_ref().ok_or("无法定位数据目录")?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("无法定位数据目录")?;
        atomic_write(path, text.as_bytes())
    }
}

/// 持久化能力装配（0b）：恢复草稿 + 会话/最近文件，同一数据目录下两个文件。
pub fn persist_env() -> PersistEnv {
    let dir = veil_ui::data_dir();
    PersistEnv {
        recovery: Box::new(FsRecovery {
            path: dir.as_ref().map(|d| d.join("recovery.dat")),
        }),
        session: Box::new(FsSession {
            path: dir.as_ref().map(|d| d.join("session.txt")),
        }),
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
