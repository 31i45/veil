//! Windows 系统外观与窗口外观（切片 17）：注册表读深色状态、DWM 深色标题栏 + Mica。
//! 全部为 Win32 FFI，零新增依赖；失败一律静默降级（内容区始终绘制不透明底色）。

use std::ffi::c_void;

use veil_egui::{NativeWindow, SystemTheme, WindowEffects};

// 注册表值读取（advapi32）：`RegGetValueW` 一步完成打开/取值/关闭。
#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(
        hkey: isize,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        ty: *mut u32,
        data: *mut c_void,
        data_size: *mut u32,
    ) -> i32;
}

// 窗口属性设置（dwmapi）。
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut c_void,
        attribute: u32,
        value: *const c_void,
        size: u32,
    ) -> i32;
}

/// HKEY_CURRENT_USER = (HKEY)0x80000001（64 位指针零扩展）。
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
/// 20H1 起为 20，更早版本为 19；两者都试，成功任一即可。
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19;
/// Win11 22H2+ 系统背景材质；值 2 = MICA（主窗口）。
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_MAINWINDOW: u32 = 2;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 系统外观：`AppsUseLightTheme=0` 即深色。值缺失/读取失败返回 None（视图层按浅色处理）。
pub struct WinSystemTheme;

impl SystemTheme for WinSystemTheme {
    fn is_dark(&mut self) -> Option<bool> {
        let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let name = wide("AppsUseLightTheme");
        let mut value: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut value as *mut u32 as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 || size != 4 {
            return None;
        }
        Some(value == 0)
    }
}

/// 窗口外观：深色标题栏 + Mica 背景材质。HWND 无效或系统不支持时静默跳过。
pub struct WinEffects;

impl WindowEffects for WinEffects {
    fn apply(&mut self, window: NativeWindow, dark: bool) {
        let hwnd = window.hwnd as *mut c_void;
        if hwnd.is_null() {
            return;
        }
        let flag = u32::from(dark);
        if set_dwm_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, flag) != 0 {
            let _ = set_dwm_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY, flag);
        }
        // Mica 只在 Win11 22H2+ 生效；失败即退化为纯色（内容区不依赖该效果）
        let _ = set_dwm_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW);
    }
}

fn set_dwm_attr(hwnd: *mut c_void, attribute: u32, value: u32) -> i32 {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            &value as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        )
    }
}
