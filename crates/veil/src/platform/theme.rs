//! Windows 系统外观与窗口外观（切片 17）：注册表读深色状态、DWM 深色标题栏
//! （深色不启用 Mica 材质，浅色保持 Mica）、高对比度检测（SPI_GETHIGHCONTRAST）。
//! 全部为 Win32 FFI，零新增依赖；失败一律静默降级（内容区始终绘制不透明底色）。

use std::ffi::c_void;

use veil_ui::{NativeWindow, SystemTheme, WindowEffects};

use super::wide;

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

// 系统参数读取（user32）：「动画效果」开关 + 高对比度状态。
#[link(name = "user32")]
extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, value: *mut c_void, win_ini: u32) -> i32;
}

// 窗口类背景画刷：GDI 创建实色画刷，窗口类持有其句柄直到被替换。
#[link(name = "gdi32")]
extern "system" {
    fn CreateSolidBrush(color: u32) -> *mut c_void;
}

#[link(name = "user32")]
extern "system" {
    fn SetClassLongPtrW(hwnd: *mut c_void, index: i32, new_long: isize) -> isize;
}

/// SPI_GETCLIENTAREAANIMATION：客户端区动画（「显示动画」开关）。
const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
/// SPI_GETHIGHCONTRAST：获取高对比度结构。
const SPI_GETHIGHCONTRAST: u32 = 0x0042;

/// HIGHCONTRASTW.cbSize。
const HIGHCONTRAST_SIZE: u32 = 12;
/// HIGHCONTRASTW.dwFlags 中的「高对比度启用」位。
const HCF_HIGHCONTRASTON: u32 = 0x00000001;

/// HKEY_CURRENT_USER = (HKEY)0x80000001（64 位指针零扩展）。
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
/// 20H1 起为 20，更早版本为 19；两者都试，成功任一即可。
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19;
/// Win11 22H2+ 系统背景材质；值 1 = NONE（纯色），2 = MICA（主窗口）。
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_NONE: u32 = 1;
const DWMSBT_MAINWINDOW: u32 = 2;

/// HIGHCONTRASTW 结构（Win32）。cbSize 仅系统使用；dwFlags 含 HCF_HIGHCONTRASTON 位。
#[repr(C)]
struct HighContrast {
    cb_size: u32,
    dw_flags: u32,
    lpsz_default_scheme: *const u16,
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

    /// 系统「显示动画」关闭时整个界面动效停用（可访问性设置，失败按开启处理）。
    fn animations_enabled(&mut self) -> bool {
        let mut enabled: i32 = 1;
        let rc = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                &mut enabled as *mut i32 as *mut c_void,
                0,
            )
        };
        if rc == 0 {
            return true;
        }
        enabled != 0
    }

    /// 检测 Windows 高对比度模式：SPI_GETHIGHCONTRAST 优先，失败则回退 COLOR_WINDOW 启发式。
    /// 两个路径都静默：任何 FFI 失败都返回 false（高对比度不启用），不影响正常外观。
    fn is_high_contrast(&mut self) -> bool {
        let mut hc = HighContrast {
            cb_size: HIGHCONTRAST_SIZE,
            dw_flags: 0,
            lpsz_default_scheme: std::ptr::null(),
        };
        let rc = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                std::mem::size_of::<HighContrast>() as u32,
                &mut hc as *mut HighContrast as *mut c_void,
                0,
            )
        };
        if rc == 0 {
            return false;
        }
        // 高对比度启用位必须被设置
        if hc.dw_flags & HCF_HIGHCONTRASTON == HCF_HIGHCONTRASTON {
            return true;
        }
        false
    }
}

/// 窗口外观：深色标题栏（深色不使用材质）+ 浅色 Mica 背景 + 窗口类背景画刷。
/// HWND 无效或系统不支持时静默跳过。
#[derive(Default)]
pub struct WinEffects {
    /// 进程内画刷缓存：COLORREF → HBRUSH。窗口类会一直引用已设置的句柄，
    /// 因此旧画刷不删除（宁可进程退出时由系统回收），避免窗口类持有悬空句柄。
    brushes: std::collections::HashMap<u32, isize>,
}

/// SetClassLongPtrW 的索引：窗口类背景画刷（GCLP_HBRBACKGROUND）。
const GCLP_HBRBACKGROUND: i32 = -10;

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
        // Mica 只在 Win11 22H2+ 生效；失败即退化为纯色（内容区不依赖该效果）。
        // 深色路径显式关闭材质：Mica 会在非客户区铺一层桌面壁纸的模糊采样，
        // 深色正文（#1E1E1E）上方于是多出一条比正文明显更亮的磨砂带（浅色下与白底接近、
        // 几乎不可见），即「深色发虚」的来源；换取锐利的纯色深色标题栏。浅色路径保持原样。
        let _ = set_dwm_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, backdrop_type(dark));
    }

    /// 未绘制区域改用主题底色擦除：winit 注册窗口类时 hbrBackground=0（黑色），
    /// 首帧覆盖前右下会露出黑块。COLORREF 的字节序是 BGR（`r | g<<8 | b<<16`）。
    fn set_background(&mut self, window: NativeWindow, color: [u8; 3]) {
        let hwnd = window.hwnd as *mut c_void;
        if hwnd.is_null() {
            return;
        }
        let colorref =
            u32::from(color[0]) | (u32::from(color[1]) << 8) | (u32::from(color[2]) << 16);
        let brush = match self.brushes.get(&colorref).copied() {
            Some(brush) => brush,
            None => {
                let handle = unsafe { CreateSolidBrush(colorref) };
                if handle.is_null() {
                    return;
                }
                let brush = handle as isize;
                self.brushes.insert(colorref, brush);
                brush
            }
        };
        // 失败（窗口类已销毁等）静默：内容区仍绘制完整背景，只是失去擦除兜底
        unsafe {
            SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, brush);
        }
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

/// 背景材质选择：深色用纯色（无材质），浅色保持 Mica。
/// 深色下 Mica 会在标题栏铺一层壁纸模糊采样，比深色正文明显更亮且带模糊结构，
/// 即「深色发虚」的来源（详见 evidence/verify/ppq-task1/findings.md）。
fn backdrop_type(dark: bool) -> u32 {
    if dark {
        DWMSBT_NONE
    } else {
        DWMSBT_MAINWINDOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 深色路径不得使用任何模糊材质；浅色路径保持 Mica 不变。
    #[test]
    fn dark_never_uses_blurring_backdrop() {
        assert_eq!(backdrop_type(true), DWMSBT_NONE);
        assert_eq!(backdrop_type(false), DWMSBT_MAINWINDOW);
    }
}
