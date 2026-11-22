//! 崩溃取证（P0-1）：panic hook + 报告原子落盘。
//!
//! release profile 为 `panic = "abort"`：hook 仍会先于 abort 执行，因此这里只做
//! 取证与落盘，不改变退出语义（不调用 `process::exit`，不吞掉原有行为）。
//! 任何写盘失败都静默忽略——不能再 panic、不能递归、不能阻断启动。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// 单份报告体积上限（截断后含截断标记）。
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
/// 报告文件名前缀。
const REPORT_PREFIX: &str = "veil-crash-";
/// 保留的最新报告份数。
const KEEP_REPORTS: usize = 10;
/// 截断标记。
const TRUNCATED_MARKER: &str = "\n[truncated]\n";

/// hook 递归保护：hook 内部再触发 panic 时立刻返回，避免无限递归。
static IN_HOOK: AtomicBool = AtomicBool::new(false);

/// 安装 panic hook（幂等）。必须在 `run()` 最早期调用，早于 CLI 解析与 UI 初始化。
pub fn install() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.set(()).is_err() {
        return;
    }
    std::panic::set_hook(Box::new(capture));
}

/// 崩溃报告目录：`data_dir()/logs`（Windows = `%APPDATA%\Veil\logs`）。
pub fn log_dir() -> Option<PathBuf> {
    veil_ui::data_dir().map(|dir| dir.join("logs"))
}

/// 原子写出报告：目录按需创建，先写 `.tmp-...` 再 rename；失败返回可读错误。
pub fn write_crash_report(dir: &Path, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = format!("{REPORT_PREFIX}{}-{}.txt", unix_ms(), std::process::id());
    let path = dir.join(&name);
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let body = clamp_report(text);
    if let Err(e) = std::fs::write(&tmp, body.as_bytes()) {
        // 写盘失败也可能留下半截临时文件，尽力清理。
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", tmp.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(path)
}

/// 只保留最新的 `keep` 份 `veil-crash-*.txt`（按文件名内嵌的 Unix 毫秒排序）。
/// 目录不可读时静默返回；单个删除失败不影响其余。
pub fn prune_crash_reports(dir: &Path, keep: usize) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut reports: Vec<(u64, PathBuf)> = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name
            .strip_prefix(REPORT_PREFIX)
            .and_then(|s| s.strip_suffix(".txt"))
        else {
            continue;
        };
        let Some(ts) = stem.split('-').next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        if entry.path().is_file() {
            reports.push((ts, entry.path()));
        }
    }
    if reports.len() <= keep {
        return;
    }
    reports.sort_by_key(|(ts, _)| *ts);
    let excess = reports.len() - keep;
    for (_, path) in reports.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

fn capture(info: &std::panic::PanicHookInfo<'_>) {
    if IN_HOOK.swap(true, Ordering::SeqCst) {
        return;
    }
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let message = payload_text(info);
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_default();
    let backtrace = std::backtrace::Backtrace::force_capture();
    let text = veil_ui::diag::render_crash(
        std::env::consts::OS,
        std::env::consts::ARCH,
        thread_name,
        &message,
        &location,
        Some(&backtrace.to_string()),
    );
    let Some(dir) = log_dir() else {
        return;
    };
    if let Ok(path) = write_crash_report(&dir, &text) {
        // 写盘失败静默；成功时把路径写到 stderr 便于脚本/自检样本定位。
        eprintln!("veil: 崩溃报告已写入 {}", path.display());
        prune_crash_reports(&dir, KEEP_REPORTS);
    }
}

/// panic 负载转文本：`&str` / `String` 常见，其余给占位而非解包失败。
fn payload_text(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = info.payload().downcast_ref::<String>() {
        return s.clone();
    }
    "<非字符串 panic 负载>".to_string()
}

/// 超过上限时按字符边界截断，并追加截断标记（总量不超过 `MAX_REPORT_BYTES`）。
fn clamp_report(text: &str) -> String {
    if text.len() <= MAX_REPORT_BYTES {
        return text.to_string();
    }
    let budget = MAX_REPORT_BYTES.saturating_sub(TRUNCATED_MARKER.len());
    let mut end = budget;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_string();
    out.push_str(TRUNCATED_MARKER);
    out
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("veil-diag-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_report_is_atomic_and_content_preserved() {
        let dir = temp_dir("write");
        let text = "=== Veil 崩溃报告 ===\n产品: Veil\n";
        let path = write_crash_report(&dir, text).expect("写出报告");
        assert!(path.starts_with(&dir));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(REPORT_PREFIX) && name.ends_with(".txt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不得残留 .tmp 文件：{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_report_errors_when_dir_is_a_file() {
        let dir = temp_dir("notdir");
        let file = dir.join("plain-file");
        std::fs::write(&file, b"x").unwrap();
        let got = write_crash_report(&file, "text");
        assert!(got.is_err(), "目录位置是文件时必须返回 Err");
        assert!(file.is_file(), "既有文件不得被破坏");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clamp_report_truncates_on_char_boundary_with_marker() {
        // 4 字节单元让预算边界落在多字节字符内部，验证回退到字符边界。
        let text = "a长".repeat(MAX_REPORT_BYTES / 4 + 16);
        let out = clamp_report(&text);
        assert!(out.len() <= MAX_REPORT_BYTES, "总量不得超过上限");
        assert!(out.ends_with(TRUNCATED_MARKER), "超过上限必须追加截断标记");
        let body = out.strip_suffix(TRUNCATED_MARKER).expect("标记必须在末尾");
        assert!(text.starts_with(body), "只允许裁掉尾部");
        assert!(text.is_char_boundary(body.len()), "必须停在字符边界");
        let budget = MAX_REPORT_BYTES - TRUNCATED_MARKER.len();
        assert!(body.len() <= budget);
        assert!(budget - body.len() < 4, "只回退到最近的字符边界");
    }

    #[test]
    fn prune_keeps_newest_reports() {
        let dir = temp_dir("prune");
        for i in 0..12u64 {
            let path = dir.join(format!("veil-crash-{:013}-42.txt", 1_700_000_000_000 + i));
            std::fs::write(&path, b"x").unwrap();
        }
        prune_crash_reports(&dir, 10);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 10, "只保留最新 10 份");
        assert!(!names.iter().any(|n| n.contains("1700000000000")));
        assert!(!names.iter().any(|n| n.contains("1700000000001")));
        assert!(names.iter().any(|n| n.contains("1700000000011")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
