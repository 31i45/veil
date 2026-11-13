//! 平台能力注入（产品层）：系统对话框、文件监听、主题配置/系统外观适配为 veil-ui 的 trait。
//! 对话框与系统外观：Windows 手写 Win32 FFI（零新增依赖），其他平台返回兜底实现。
//! 文件监听：notify 事件线程（切片 16），跨平台同一实现。配置读写：std 原子写（切片 17）。

mod assoc;
mod clock;
mod error;
pub mod http; // 远程图片抓取（v1.4）：WinHTTP 手写 FFI
pub mod ipc;
mod pdf; // 原生 PDF 导出（Wave 3A，跨平台 crate 但字体链 Windows 优先）
#[cfg(windows)]
mod pdf_layout; // Wave 4：Markdown → PDF 版面（纯逻辑，可单测）
#[cfg(windows)]
mod pdf_media; // Wave 4：图片（WIC）/ 公式（latex-rust）→ RGB
#[cfg(windows)]
mod print;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod theme;
mod watch;
#[cfg(windows)]
mod wic;
#[cfg(windows)]
mod windows;

pub use assoc::{parse_command, run as run_assoc};
pub use error::{open_failure_text, show_error, show_info, OpenFailure};

use std::path::PathBuf;

use veil_ui::i18n::{t, tf, Key};
#[cfg(not(windows))]
use veil_ui::NativeWindow;
use veil_ui::{
    ConfigStore, FileDialog, FileWatcher, ImageData, PersistEnv, RecoveryStore, SessionStore,
    SnapshotMeta, SnapshotStore, SystemTheme, ThemeEnv, WindowEffects,
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
    Err(t(Key::ErrUnsupportedImage).into())
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
        Err(t(Key::ErrUrlUnsupported).into())
    }
}

/// 主题配置读写：`%APPDATA%\Veil\config.toml`，原子写（同目录 tmp + rename）并按需建目录。
/// 文件不存在属正常状态（首次运行），`load` 返回 Err 由视图层静默使用默认配置。
pub struct FsConfig {
    pub path: Option<PathBuf>,
}

impl ConfigStore for FsConfig {
    fn load(&mut self) -> Result<String, String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoConfigDir).to_string())?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // 非法 UTF-8 不整文件拒绝：逐行解析时非法值会被忽略并提示（容错优于失效）
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn save(&mut self, text: &str) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoConfigDir).to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                tf(
                    Key::ErrIoCreateDir,
                    &[&e.to_string(), &dir.display().to_string()],
                )
            })?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".veil-tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| {
                tf(
                    Key::ErrIoWriteFile,
                    &[&e.to_string(), &path.display().to_string()],
                )
            })
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
        callouts: callouts(),
    }
}

/// 自定义 Callout 配置路径：`%APPDATA%\Veil\callouts.json`（与 theme 配置同根）。
pub fn callouts_path() -> Option<PathBuf> {
    veil_ui::data_dir().map(|d| d.join("callouts.json"))
}

/// 读取自定义 Callout 表。文件缺失/不可读/JSON 非法 → 空表（静默，仅内置五种生效）。
pub fn callouts() -> veil_core::CalloutRegistry {
    callouts_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| veil_core::CalloutRegistry::from_json(&t))
        .unwrap_or_default()
}

/// 原子写小助手：同目录 tmp + rename，并按需创建父目录（config/recovery/session 共用）。
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            tf(
                Key::ErrIoCreateDir,
                &[&e.to_string(), &dir.display().to_string()],
            )
        })?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".veil-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            tf(
                Key::ErrIoWriteFile,
                &[&e.to_string(), &path.display().to_string()],
            )
        })
}

/// 崩溃恢复草稿（0b）：`<数据目录>/recovery.dat` 单槽，原子写；清除时文件不存在视为成功。
pub struct FsRecovery {
    pub path: Option<PathBuf>,
}

impl RecoveryStore for FsRecovery {
    fn load(&mut self) -> Result<Vec<u8>, String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoDataDir).to_string())?;
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }
    fn save(&mut self, data: &[u8]) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoDataDir).to_string())?;
        atomic_write(path, data)
    }
    fn clear(&mut self) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoDataDir).to_string())?;
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
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoDataDir).to_string())?;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| t(Key::ErrNoDataDir).to_string())?;
        atomic_write(path, text.as_bytes())
    }
}

/// 版本历史快照目录（N1/B8a）：`<数据目录>/snapshots/<stable_key_hex>/`
/// 稳定键 = 规范化绝对路径哈希（未命名草稿为临时会话键），内容变化不换目录。
/// 每槽两份文件：`<unix_ms>.md` 快照正文 + `<unix_ms>.meta` 元数据
/// （哈希/字节数，供去重与损坏校验），均原子写。
pub struct FsSnapshotStore {
    /// 数据目录下的 snapshots 根目录（不存在时首次 save 自动创建）。
    root: PathBuf,
    /// 每文档保留最近 N 槽。
    keep_newest: usize,
    /// 全局容量上限（字节），超出按最旧优先 LRU 淘汰。
    max_total_bytes: u64,
}

/// 每文档快照上限：50 份。
const SNAPSHOT_KEEP_NEWEST: usize = 50;
/// 全局快照容量上限：200 MB。
const SNAPSHOT_MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;

impl FsSnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            keep_newest: SNAPSHOT_KEEP_NEWEST,
            max_total_bytes: SNAPSHOT_MAX_TOTAL_BYTES,
        }
    }

    /// 测试专用：收紧上限以验证淘汰策略（不进入生产装配路径）。
    #[cfg(test)]
    fn with_limits(root: PathBuf, keep_newest: usize, max_total_bytes: u64) -> Self {
        Self {
            root,
            keep_newest,
            max_total_bytes,
        }
    }

    fn dir_of(&self, key: u64) -> PathBuf {
        self.root.join(format!("{key:016x}"))
    }

    fn slot_paths(&self, key: u64, ts: u64) -> (PathBuf, PathBuf) {
        let dir = self.dir_of(key);
        (
            dir.join(format!("{ts:016x}.md")),
            dir.join(format!("{ts:016x}.meta")),
        )
    }

    /// 读取元数据文件（三行文本：`v1` / `hash` / `bytes`）。
    fn read_meta_file(path: &std::path::Path) -> Option<(u64, u64)> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        if lines.next()? != "v1" {
            return None;
        }
        let hash = u64::from_str_radix(lines.next()?.trim(), 16).ok()?;
        let bytes = lines.next()?.trim().parse::<u64>().ok()?;
        Some((hash, bytes))
    }

    fn write_meta_file(
        dir: &std::path::Path,
        ts: u64,
        hash: u64,
        bytes: u64,
    ) -> Result<(), String> {
        let meta = dir.join(format!("{ts:016x}.meta"));
        atomic_write(&meta, format!("v1\n{hash:016x}\n{bytes}\n").as_bytes())
    }

    /// 扫描一个目录的全部有效槽：正文可读且元数据可得（缺失/损坏时按正文重算并回写）。
    /// 读不到正文的槽视为损坏，直接跳过。
    fn scan_dir(&self, key: u64) -> Vec<SnapshotMeta> {
        let dir = self.dir_of(key);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".md") else {
                continue;
            };
            let Ok(ts) = u64::from_str_radix(stem, 16) else {
                continue;
            };
            let md_path = entry.path();
            if !md_path.is_file() {
                continue;
            }
            let Ok(data) = std::fs::read(&md_path) else {
                continue; // 正文不可读：损坏/缺失，跳过
            };
            if std::str::from_utf8(&data).is_err() {
                continue; // 非 UTF-8 文本快照：损坏，跳过而不是崩溃
            }
            let actual_hash = veil_ui::content_hash(&data);
            let actual_bytes = data.len() as u64;
            let meta_path = dir.join(format!("{stem}.meta"));
            let valid = Self::read_meta_file(&meta_path)
                .filter(|&(h, b)| h == actual_hash && b == actual_bytes);
            let (hash, bytes) = match valid {
                Some(v) => v,
                None => {
                    // 旧格式/元数据缺失或与正文不符：按正文重建，失败不影响列表
                    let _ = Self::write_meta_file(&dir, ts, actual_hash, actual_bytes);
                    (actual_hash, actual_bytes)
                }
            };
            out.push(SnapshotMeta {
                ts_ms: ts,
                hash,
                bytes,
            });
        }
        out.sort_by_key(|m| std::cmp::Reverse(m.ts_ms));
        out
    }

    /// 全局已用字节数（仅统计有效 `.md`；上限按正文容量计）。
    fn total_bytes(&self) -> u64 {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        let mut total = 0u64;
        for dir in rd.flatten() {
            let Ok(inner) = std::fs::read_dir(dir.path()) else {
                continue;
            };
            for f in inner.flatten() {
                let name = f.file_name();
                if name.to_string_lossy().ends_with(".md") {
                    total += f.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        total
    }

    /// 全局 LRU：按时间戳最旧优先删除，直到回到容量上限内。返回删除份数。
    fn enforce_global_budget(&self) -> usize {
        if self.total_bytes() <= self.max_total_bytes {
            return 0;
        }
        let mut slots: Vec<(u64, PathBuf)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.root) {
            for dir in rd.flatten() {
                let Ok(inner) = std::fs::read_dir(dir.path()) else {
                    continue;
                };
                for f in inner.flatten() {
                    let name = f.file_name();
                    let Some(stem) = name
                        .to_string_lossy()
                        .strip_suffix(".md")
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    let Ok(ts) = u64::from_str_radix(&stem, 16) else {
                        continue;
                    };
                    slots.push((ts, f.path()));
                }
            }
        }
        slots.sort_by_key(|(ts, _)| *ts);
        let mut total = self.total_bytes();
        let mut removed = 0;
        for (ts, path) in slots {
            if total <= self.max_total_bytes {
                break;
            }
            let len = path.metadata().map(|m| m.len()).unwrap_or(0);
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                removed += 1;
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_file(parent.join(format!("{ts:016x}.meta")));
                }
            }
        }
        removed
    }
}

impl SnapshotStore for FsSnapshotStore {
    fn save_snapshot(
        &mut self,
        doc_key: u64,
        snapshot_text: &str,
    ) -> Result<veil_ui::SnapshotMeta, String> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let data = snapshot_text.as_bytes();
        let hash = veil_ui::content_hash(data);
        let bytes = data.len() as u64;
        // 同一内容不重复写：已有任一槽哈希相同即复用（历史按时间倒序，取最新那份）
        if let Some(existing) = self.scan_dir(doc_key).into_iter().find(|m| m.hash == hash) {
            return Ok(existing);
        }
        let dir = self.dir_of(doc_key);
        std::fs::create_dir_all(&dir)
            .map_err(|e| tf(Key::ErrSnapshotDirCreate, &[&e.to_string()]))?;
        let mut ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .map_err(|e| e.to_string())?;
        // 毫秒级同刻冲突：递增到未占用槽名，保证目录内唯一
        while dir.join(format!("{ts:016x}.md")).exists() {
            ts += 1;
        }
        let (md_path, _) = self.slot_paths(doc_key, ts);
        atomic_write(&md_path, data).map_err(|e| tf(Key::ErrSnapshotWrite, &[&e]))?;
        if let Err(e) = Self::write_meta_file(&dir, ts, hash, bytes) {
            // 元数据失败不丢正文：列表会按正文重建元数据
            eprintln!("快照元数据写入失败（将按正文重建）: {e}");
        }
        // 单文档上限 → 全局容量上限（LRU 最旧优先）
        let _ = self.prune(doc_key, self.keep_newest);
        let _ = self.enforce_global_budget();
        Ok(veil_ui::SnapshotMeta {
            ts_ms: ts,
            hash,
            bytes,
        })
    }

    fn list_snapshots(&mut self, doc_key: u64) -> Result<Vec<veil_ui::SnapshotMeta>, String> {
        Ok(self.scan_dir(doc_key))
    }

    fn read_snapshot(&mut self, doc_key: u64, ts_ms: u64) -> Result<String, String> {
        let (md_path, meta_path) = self.slot_paths(doc_key, ts_ms);
        let data = std::fs::read(&md_path).map_err(|e| {
            tf(
                Key::ErrSnapshotRead,
                &[&md_path.display().to_string(), &e.to_string()],
            )
        })?;
        if let Some((hash, bytes)) = Self::read_meta_file(&meta_path) {
            if hash != veil_ui::content_hash(&data) || bytes != data.len() as u64 {
                return Err(t(Key::ErrSnapshotMeta).into());
            }
        }
        String::from_utf8(data).map_err(|e| tf(Key::ErrSnapshotUtf8, &[&e.to_string()]))
    }

    fn delete_snapshot(&mut self, doc_key: u64, ts_ms: u64) -> Result<(), String> {
        let (md_path, meta_path) = self.slot_paths(doc_key, ts_ms);
        for path in [md_path, meta_path] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(tf(
                        Key::ErrSnapshotDelete,
                        &[&path.display().to_string(), &e.to_string()],
                    ))
                }
            }
        }
        Ok(())
    }

    fn clear_snapshots(&mut self, doc_key: u64) -> Result<usize, String> {
        let dir = self.dir_of(doc_key);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in rd.flatten() {
            let is_snapshot = entry.file_name().to_string_lossy().ends_with(".md");
            if entry.path().is_file() {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed += usize::from(is_snapshot),
                    Err(e) => return Err(tf(Key::ErrHistoryClear, &[&e.to_string()])),
                }
            }
        }
        Ok(removed)
    }

    fn prune(&mut self, doc_key: u64, keep_newest: usize) -> Result<usize, String> {
        let mut all = self.scan_dir(doc_key);
        if all.len() <= keep_newest {
            return Ok(0);
        }
        // scan_dir 已倒序：保留前 keep_newest，删除其余
        let mut removed = 0;
        for meta in all.drain(keep_newest..) {
            self.delete_snapshot(doc_key, meta.ts_ms)?;
            removed += 1;
        }
        Ok(removed)
    }

    fn migrate_key(&mut self, from: u64, to: u64) -> Result<usize, String> {
        if from == to {
            return Ok(0);
        }
        let src = self.dir_of(from);
        let dst = self.dir_of(to);
        if !src.is_dir() {
            return Ok(0);
        }
        std::fs::create_dir_all(&dst)
            .map_err(|e| tf(Key::ErrSnapshotDirCreate, &[&e.to_string()]))?;
        let existing: Vec<u64> = self.scan_dir(to).into_iter().map(|m| m.hash).collect();
        let mut moved = 0;
        for meta in self.scan_dir(from) {
            let (md, metap) = self.slot_paths(from, meta.ts_ms);
            // 目标已有同内容快照：源槽去重删除（不保留重复版本）
            if existing.contains(&meta.hash) {
                let _ = std::fs::remove_file(&md);
                let _ = std::fs::remove_file(&metap);
                continue;
            }
            let mut ts = meta.ts_ms;
            while dst.join(format!("{ts:016x}.md")).exists() {
                ts += 1;
            }
            let (to_md, to_meta) = self.slot_paths(to, ts);
            if let Err(e) = std::fs::rename(&md, &to_md) {
                return Err(tf(Key::ErrSnapshotMigrate, &[&e.to_string()]));
            }
            if std::fs::rename(&metap, &to_meta).is_err() {
                let _ = Self::write_meta_file(&dst, ts, meta.hash, meta.bytes);
            }
            moved += 1;
        }
        let _ = std::fs::remove_dir(&src); // 空目录顺手清理；非空则保留
        Ok(moved)
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("veil-b8a-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dedupe_does_not_rewrite_same_content() {
        let root = temp_root("dedupe");
        let mut s = FsSnapshotStore::new(root.clone());
        let a = s.save_snapshot(7, "同一内容\n").unwrap();
        let b = s.save_snapshot(7, "同一内容\n").unwrap();
        assert_eq!(a, b, "同内容必须复用既有槽");
        assert_eq!(s.list_snapshots(7).unwrap().len(), 1);
        let c = s.save_snapshot(7, "另一内容\n").unwrap();
        assert_ne!(a.ts_ms, c.ts_ms);
        assert_eq!(s.list_snapshots(7).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn per_doc_prune_keeps_newest() {
        let root = temp_root("prune");
        let mut s = FsSnapshotStore::with_limits(root.clone(), 3, u64::MAX);
        for t in ["v1", "v2", "v3", "v4", "v5"] {
            s.save_snapshot(1, t).unwrap();
        }
        let list = s.list_snapshots(1).unwrap();
        assert_eq!(list.len(), 3);
        let texts: Vec<String> = list
            .iter()
            .map(|m| s.read_snapshot(1, m.ts_ms).unwrap())
            .collect();
        assert_eq!(texts, vec!["v5", "v4", "v3"], "最新在前，旧版本被淘汰");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn global_budget_evicts_oldest_lru() {
        let root = temp_root("lru");
        // 每份 5 字节，全局上限 10 字节：第三份写入后必须淘汰最旧一份
        let mut s = FsSnapshotStore::with_limits(root.clone(), 50, 10);
        s.save_snapshot(1, "aaaaa").unwrap();
        s.save_snapshot(1, "bbbbb").unwrap();
        s.save_snapshot(1, "ccccc").unwrap();
        let list = s.list_snapshots(1).unwrap();
        assert_eq!(list.len(), 2, "全局容量必须被守住");
        assert!(s.total_bytes() <= 10);
        let texts: Vec<String> = list
            .iter()
            .map(|m| s.read_snapshot(1, m.ts_ms).unwrap())
            .collect();
        assert_eq!(texts, vec!["ccccc", "bbbbb"], "最旧版本先淘汰");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_snapshot_is_skipped_not_panic() {
        let root = temp_root("corrupt");
        let mut s = FsSnapshotStore::new(root.clone());
        let good = s.save_snapshot(3, "good\n").unwrap();
        let dir = root.join(format!("{:016x}", 3u64));
        // 非 UTF-8 正文：列表必须跳过，读取返回错误
        std::fs::write(
            dir.join(format!("{:016x}.md", good.ts_ms + 1)),
            [0xff, 0xfe, 0x00],
        )
        .unwrap();
        let list = s.list_snapshots(3).unwrap();
        assert_eq!(list.len(), 1, "损坏槽被跳过");
        assert_eq!(list[0].ts_ms, good.ts_ms);
        assert!(s.read_snapshot(3, good.ts_ms + 1).is_err());
        // 缺失槽：读取返回可读错误，不崩溃
        assert!(s.read_snapshot(3, 999_999).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn text_roundtrip_preserves_bytes_and_meta() {
        let root = temp_root("roundtrip");
        let mut s = FsSnapshotStore::new(root.clone());
        let text = "# 标题\r\n\n中文 🚀 \0 tail\n";
        let meta = s.save_snapshot(9, text).unwrap();
        assert_eq!(meta.bytes, text.len() as u64);
        assert_eq!(meta.hash, veil_ui::content_hash(text.as_bytes()));
        assert_eq!(s.read_snapshot(9, meta.ts_ms).unwrap(), text);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn migrate_moves_and_dedupes() {
        let root = temp_root("migrate");
        let mut s = FsSnapshotStore::new(root.clone());
        s.save_snapshot(11, "draft-v1").unwrap();
        s.save_snapshot(11, "draft-v2").unwrap();
        s.save_snapshot(22, "draft-v2").unwrap(); // 目标已有同内容
        let moved = s.migrate_key(11, 22).unwrap();
        assert_eq!(moved, 1, "同内容去重，只迁移一份");
        let mut texts: Vec<String> = s
            .list_snapshots(22)
            .unwrap()
            .iter()
            .map(|m| s.read_snapshot(22, m.ts_ms).unwrap())
            .collect();
        texts.sort();
        assert_eq!(texts, vec!["draft-v1", "draft-v2"]);
        assert!(s.list_snapshots(11).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_and_clear_are_idempotent() {
        let root = temp_root("delete");
        let mut s = FsSnapshotStore::new(root.clone());
        let m = s.save_snapshot(5, "x").unwrap();
        s.delete_snapshot(5, m.ts_ms).unwrap();
        s.delete_snapshot(5, m.ts_ms).unwrap(); // 已不存在：成功
        assert!(s.list_snapshots(5).unwrap().is_empty());
        s.save_snapshot(5, "y").unwrap();
        s.save_snapshot(5, "z").unwrap();
        assert_eq!(s.clear_snapshots(5).unwrap(), 2);
        assert!(s.list_snapshots(5).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}

/// 持久化能力装配（0b）：恢复草稿 + 会话/最近文件 + 版本快照，同一数据目录下。
pub fn persist_env() -> PersistEnv {
    let dir = veil_ui::data_dir();
    let root = dir
        .as_ref()
        .map(|d| d.join("snapshots"))
        .unwrap_or_default();
    PersistEnv {
        recovery: Box::new(FsRecovery {
            path: dir.as_ref().map(|d| d.join("recovery.dat")),
        }),
        session: Box::new(FsSession {
            path: dir.as_ref().map(|d| d.join("session.txt")),
        }),
        snapshot: Box::new(FsSnapshotStore::new(root)),
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
        assert_eq!(
            w.iter().position(|&c| c == 0),
            Some(1),
            "首个 NUL 即为 Win32 可见边界"
        );
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
