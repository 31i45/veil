//! 打开失败的用户可见反馈（信任显性化）：可读中文说明 + 手写 `MessageBoxW`。
//! 发布构建是 GUI 子系统（无控制台），`eprintln!` 无人可见，故失败必须弹框。

use std::ffi::c_void;
use std::path::Path;

/// 打开失败的可归因原因：决定文案里的原因与「下一步」。
pub enum OpenFailure {
    /// 文件不是合法 UTF-8（GBK 等本地编码最常见）。
    NotUtf8(String),
    /// 文件不存在或已被移动/删除。
    NotFound(String),
    /// 无读取权限（只读/受控文件夹/安全软件拦截）。
    Denied(String),
    /// 其他 IO 失败：保留原始诊断。
    Other(String),
}

impl OpenFailure {
    /// 标准 IO 错误 → 归因（原始错误文本作为诊断保留，不吞细节）。
    pub fn from_io(e: &std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => OpenFailure::NotFound(e.to_string()),
            std::io::ErrorKind::PermissionDenied => OpenFailure::Denied(e.to_string()),
            _ => OpenFailure::Other(e.to_string()),
        }
    }
}

/// 打开失败说明：包含完整路径、可读原因与可执行的下一步（纯函数，便于测试）。
pub fn open_failure_text(path: &Path, failure: &OpenFailure) -> String {
    let (reason, next) = match failure {
        OpenFailure::NotUtf8(e) => (
            format!("文件不是有效的 UTF-8 编码（{e}）"),
            "请将文件另存为 UTF-8 后重试。",
        ),
        OpenFailure::NotFound(e) => (
            format!("文件不存在或已被移动（{e}）"),
            "请确认路径后重试，或从「文件 → 打开」重新选择。",
        ),
        OpenFailure::Denied(e) => (
            format!("没有读取该文件的权限（{e}）"),
            "请确认文件未被只读锁定或被安全软件拦截，取得权限后重试。",
        ),
        OpenFailure::Other(e) => (format!("读取失败（{e}）"), "请确认文件可访问后重试。"),
    };
    format!(
        "无法打开：{}\n\n原因：{}\n\n下一步：{}",
        path.display(),
        reason,
        next
    )
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, ty: u32) -> i32;
}

/// 弹出模态错误框（阻塞至用户点「确定」）。
#[cfg(windows)]
pub fn show_error(title: &str, text: &str) {
    const MB_OK: u32 = 0x0000_0000;
    const MB_ICONERROR: u32 = 0x0000_0010;
    let text = super::wide(text);
    let title = super::wide(title);
    // SAFETY: 两个指针都指向以 NUL 结尾的 UTF-16 缓冲，调用期间均有效
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// 非 Windows 兜底：本仓库仅发布 Windows 构建，其余平台保留 stderr 诊断。
#[cfg(not(windows))]
pub fn show_error(title: &str, text: &str) {
    eprintln!("{title}: {text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> &'static Path {
        Path::new("D:/docs/笔记.md")
    }

    #[test]
    fn not_utf8_text_has_path_and_next_step() {
        let t = open_failure_text(path(), &OpenFailure::NotUtf8("非法字节".into()));
        assert!(t.contains("笔记.md"), "须包含路径: {t}");
        assert!(t.contains("UTF-8"), "须指明编码原因: {t}");
        assert!(t.contains("另存为 UTF-8"), "须给出可执行下一步: {t}");
    }

    #[test]
    fn missing_file_text_has_path_and_next_step() {
        let t = open_failure_text(
            path(),
            &OpenFailure::NotFound("系统找不到指定的文件。".into()),
        );
        assert!(t.contains("笔记.md"), "须包含路径: {t}");
        assert!(t.contains("不存在"), "须指明原因: {t}");
        assert!(t.contains("重试"), "须给出可执行下一步: {t}");
    }

    #[test]
    fn permission_denied_text_has_path_and_next_step() {
        let t = open_failure_text(path(), &OpenFailure::Denied("拒绝访问。".into()));
        assert!(t.contains("笔记.md"), "须包含路径: {t}");
        assert!(t.contains("权限"), "须指明原因: {t}");
        assert!(t.contains("重试"), "须给出可执行下一步: {t}");
    }

    /// IO 错误归因：只读/权限与缺失各自落到对应分支，其余归 Other（不丢诊断）。
    #[test]
    fn io_error_kinds_are_classified() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(OpenFailure::from_io(&denied), OpenFailure::Denied(_)));
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(OpenFailure::from_io(&missing), OpenFailure::NotFound(_)));
        let other = std::io::Error::from(std::io::ErrorKind::Other);
        assert!(matches!(OpenFailure::from_io(&other), OpenFailure::Other(_)));
    }

    /// 第四类归因（其他 IO 失败）：保留原始诊断，并仍给路径与下一步。
    #[test]
    fn other_failure_text_keeps_diagnostic_and_next_step() {
        let t = open_failure_text(path(), &OpenFailure::Other("句柄无效。".into()));
        assert!(t.contains("笔记.md"), "须包含路径: {t}");
        assert!(t.contains("读取失败"), "须指明原因: {t}");
        assert!(t.contains("句柄无效。"), "须保留原始诊断: {t}");
        assert!(t.contains("下一步"), "须给出可执行下一步: {t}");
    }

    /// 路径含非 ASCII 与空格时原样呈现，不被转义或截断。
    #[test]
    fn text_preserves_non_ascii_path_verbatim() {
        let p = Path::new("D:/我的 文档/草稿 (1).md");
        let t = open_failure_text(p, &OpenFailure::NotFound("系统找不到指定的文件。".into()));
        assert!(t.contains("我的 文档/草稿 (1).md"), "路径须原样: {t}");
    }
}
