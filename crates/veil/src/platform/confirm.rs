//! 未保存内容确认（3e 修 B）：Win32 MessageBoxW 三选一（手写 FFI，零新增依赖）。
//! 按钮为系统本地化的「是/否/取消」：是=保存、否=放弃、取消=返回编辑。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::path::Path;

use veil_ui::{Confirmer, UnsavedChoice};

type Hwnd = *mut c_void;

#[link(name = "user32")]
extern "system" {
    fn GetActiveWindow() -> Hwnd;
    fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, u_type: u32) -> i32;
}

const MB_YESNOCANCEL: u32 = 0x0000_0003;
const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_DEFBUTTON1: u32 = 0x0000_0000;
const IDYES: i32 = 6;
const IDNO: i32 = 7;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct WinConfirm;

impl Confirmer for WinConfirm {
    fn confirm_unsaved(&mut self, file: Option<&Path>) -> UnsavedChoice {
        let name = file
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("未命名草稿");
        let text = wide(&format!(
            "「{name}」有未保存的修改。\n\n是：保存后继续\n否：放弃修改\n取消：返回编辑"
        ));
        let caption = wide("Veil");
        let r = unsafe {
            MessageBoxW(
                GetActiveWindow(),
                text.as_ptr(),
                caption.as_ptr(),
                MB_YESNOCANCEL | MB_ICONWARNING | MB_DEFBUTTON1,
            )
        };
        match r {
            IDYES => UnsavedChoice::Save,
            IDNO => UnsavedChoice::Discard,
            _ => UnsavedChoice::Cancel,
        }
    }
}
