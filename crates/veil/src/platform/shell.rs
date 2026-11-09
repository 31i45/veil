//! 用系统默认程序打开链接（手写 Win32 FFI，零新增依赖；同 print.rs 风格）。
//! 只做「把已校验的目标交给 shell」这一件事：协议白名单的事实来源是
//! `veil_ui::url_is_openable`，这里再校验一次，避免端口被误用时把
//! `file:`/`javascript:` 之类的目标交给系统执行。

#![allow(non_snake_case)]

use std::ffi::c_void;

use veil_ui::UrlOpener;

use super::wide;

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut c_void,
        op: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> isize;
}

/// SW_SHOWNORMAL：交给 shell 的关联程序按其默认窗口状态显示。
const SW_SHOWNORMAL: i32 = 1;

/// 返回值 <= 32 是 ShellExecuteW 约定的错误码区间（> 32 才是成功句柄）。
const SHELL_EXECUTE_MIN_SUCCESS: isize = 32;

/// 系统默认程序（默认浏览器/邮件客户端）打开 URL。
pub struct SystemUrlOpener;

impl UrlOpener for SystemUrlOpener {
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        if !veil_ui::url_is_openable(url) {
            return Err(format!("不支持的链接协议: {url}"));
        }
        let op = wide("open");
        let file = wide(url);
        // SAFETY: 两个 UTF-16 缓冲区在调用期间存活且以 NUL 结尾；
        // 其余参数按 Win32 约定传空（无参数/无工作目录/无父窗口）。
        let ret = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if ret <= SHELL_EXECUTE_MIN_SUCCESS {
            return Err(format!("ShellExecuteW 返回 {ret}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白名单外的目标必须在调用 shell32 之前拒绝（测试不实际弹浏览器）。
    #[test]
    fn rejects_non_whitelisted_scheme_without_invoking_shell() {
        let mut o = SystemUrlOpener;
        for bad in ["file:///C:/Windows/notepad.exe", "javascript:alert(1)", "a.md"] {
            let err = o.open_url(bad).expect_err("白名单外必须拒绝");
            assert!(err.contains("不支持的链接协议"), "错误信息可读: {err}");
        }
    }
}
