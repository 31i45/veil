//! 平台能力注入（产品层）：系统对话框、文件监听、主题配置/系统外观适配为 veil-ui 的 trait。
//! 对话框与系统外观：Windows 手写 Win32 FFI（零新增依赖），其他平台返回兜底实现。
//! 文件监听：notify 事件线程（切片 16），跨平台同一实现。配置读写：std 原子写（切片 17）。

#[cfg(windows)]
mod print;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod theme;
mod clock;
mod error;
mod watch;
#[cfg(windows)]
mod wic;
#[cfg(windows)]
mod windows;

pub use error::{open_failure_text, show_error, OpenFailure};

use std::path::PathBuf;

#[cfg(not(windows))]
use veil_ui::NativeWindow;
use veil_ui::{
    ConfigStore, FileDialog, FileWatcher, ImageData, PersistEnv, RecoveryStore, SessionStore,
    SystemTheme, ThemeEnv, WindowEffects,
};

/// UTF-16 + NUL 结尾：Win32 W 系列 API 唯一接受的字符串形态。
/// 各 Win32 子模块共用此一份实现（等价于 `OsStr::encode_wide`，但无需额外 trait）。
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

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

/// URL 打开能力装配：Windows 走 ShellExecuteW（系统默认程序）；
/// 其他平台为「不支持」兜底（本仓库仅支持 Windows，兜底只为保持编译整洁）。
pub fn url_opener() -> Box<dyn veil_ui::UrlOpener> {
    #[cfg(windows)]
    {
        Box::new(shell::SystemUrlOpener)
    }
    #[cfg(not(windows))]
    {
        Box::new(NoUrlOpener)
    }
}

/// 系统时钟装配（信任显性化）：状态栏「已保存 HH:MM」的时间来源。
/// 与 theme_env 同模式——本层是实现细节，veil-ui 只见 `Clock` trait。
pub fn system_clock() -> Box<dyn veil_ui::Clock> {
    Box::new(clock::SystemClock)
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
struct NoUrlOpener;
#[cfg(not(windows))]
impl veil_ui::UrlOpener for NoUrlOpener {
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Err("当前平台不支持打开链接".into())
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

    /// 空串也必须带 NUL 终止符（Win32 W API 不接受空缓冲）。
    #[cfg(windows)]
    #[test]
    fn wide_empty_string_is_single_terminator() {
        assert_eq!(wide(""), vec![0u16]);
    }

    /// 非 ASCII（中文 + 补充平面）按 UTF-16 code unit 展开，末尾补一个 NUL。
    #[cfg(windows)]
    #[test]
    fn wide_encodes_non_ascii_code_units() {
        let w = wide("a中😀");
        assert_eq!(w, vec![0x61, 0x4E2D, 0xD83D, 0xDE00, 0]);
    }

    /// 内嵌 U+0000：实现保留原字符并再补终止符，Win32 实际只读到首个 NUL（下标 1）。
    #[cfg(windows)]
    #[test]
    fn wide_keeps_embedded_nul_before_appended_terminator() {
        let w = wide("a\0b");
        assert_eq!(w, vec![0x61, 0, 0x62, 0]);
        assert_eq!(w.iter().position(|&c| c == 0), Some(1), "首个 NUL 即为 Win32 可见边界");
    }

    /// 长串：每个 char 的 UTF-16 单元数 + 1 个终止符，不截断、不丢尾部。
    #[cfg(windows)]
    #[test]
    fn wide_long_input_ends_with_single_terminator() {
        let s = "长".repeat(5_000) + &"x".repeat(1_000);
        let w = wide(&s);
        assert_eq!(w.len(), s.encode_utf16().count() + 1);
        assert_eq!(w.len(), 6_001);
        assert_eq!(w[0], 0x957F);
        assert_eq!(*w.last().unwrap(), 0);
    }
}
