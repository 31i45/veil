//! Win32 通用文件对话框（手写 FFI，零新增依赖）。
//! 仅用 VISTA+ 稳定的 OPENFILENAMEW 布局；`GetActiveWindow` 让对话框归属当前窗口。

#![allow(non_snake_case)]

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use veil_egui::FileDialog;

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Lpstr = *mut u16;
type Lpcwstr = *const u16;
type Lpvoid = *mut c_void;
type Lparam = isize;

/// OPENFILENAMEW（commdlg.h，VISTA+ 字段顺序；repr(C) 保证 32/64 位对齐）。
#[repr(C)]
#[allow(dead_code)]
struct OpenFileNameW {
    l_struct_size: u32,
    hwnd_owner: Hwnd,
    h_instance: Hinstance,
    lpstr_filter: Lpcwstr,
    lpstr_custom_filter: Lpstr,
    n_max_cust_filter: u32,
    n_filter_index: u32,
    lpstr_file: Lpstr,
    n_max_file: u32,
    lpstr_file_title: Lpstr,
    n_max_file_title: u32,
    lpstr_initial_dir: Lpcwstr,
    lpstr_title: Lpcwstr,
    flags: u32,
    n_file_offset: u16,
    n_file_extension: u16,
    lpstr_def_ext: Lpcwstr,
    l_cust_data: Lparam,
    lpfn_hook: Lpvoid,
    lp_template_name: Lpcwstr,
    pv_reserved: Lpvoid,
    dw_reserved: u32,
    flags_ex: u32,
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(ofn: *mut OpenFileNameW) -> i32;
    fn GetSaveFileNameW(ofn: *mut OpenFileNameW) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetActiveWindow() -> Hwnd;
}

const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
const OFN_NOCHANGEDIR: u32 = 0x0000_0004;
const OFN_HIDEREADONLY: u32 = 0x0000_0008;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_EXPLORER: u32 = 0x0008_0000;

const MAX_PATH_U16: usize = 32_768;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 过滤器串：每对「描述\0模式\0」，整体以额外 \0 结尾。
fn filter() -> Vec<u16> {
    "Markdown (*.md;*.markdown)\0*.md;*.markdown\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect()
}

fn run(flags: u32, title: &str, suggested: Option<&str>) -> Option<PathBuf> {
    let filter = filter();
    let title = wide(title);
    let def_ext = wide("md");
    let mut buf = [0u16; MAX_PATH_U16];
    if let Some(name) = suggested {
        for (dst, src) in buf.iter_mut().zip(name.encode_utf16()) {
            *dst = src;
        }
    }

    let mut ofn = OpenFileNameW {
        l_struct_size: std::mem::size_of::<OpenFileNameW>() as u32,
        hwnd_owner: unsafe { GetActiveWindow() },
        h_instance: std::ptr::null_mut(),
        lpstr_filter: filter.as_ptr(),
        lpstr_custom_filter: std::ptr::null_mut(),
        n_max_cust_filter: 0,
        n_filter_index: 1, // 1-based：默认 Markdown 过滤器
        lpstr_file: buf.as_mut_ptr(),
        n_max_file: MAX_PATH_U16 as u32,
        lpstr_file_title: std::ptr::null_mut(),
        n_max_file_title: 0,
        lpstr_initial_dir: std::ptr::null(),
        lpstr_title: title.as_ptr(),
        flags,
        n_file_offset: 0,
        n_file_extension: 0,
        lpstr_def_ext: def_ext.as_ptr(),
        l_cust_data: 0,
        lpfn_hook: std::ptr::null_mut(),
        lp_template_name: std::ptr::null(),
        pv_reserved: std::ptr::null_mut(),
        dw_reserved: 0,
        flags_ex: 0,
    };

    let ok = unsafe {
        if suggested.is_some() {
            GetSaveFileNameW(&mut ofn)
        } else {
            GetOpenFileNameW(&mut ofn)
        }
    };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

pub struct WinDialogs;

impl FileDialog for WinDialogs {
    fn open_markdown(&mut self) -> Option<PathBuf> {
        run(
            OFN_EXPLORER
                | OFN_FILEMUSTEXIST
                | OFN_PATHMUSTEXIST
                | OFN_HIDEREADONLY
                | OFN_NOCHANGEDIR,
            "打开 Markdown",
            None,
        )
    }

    fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf> {
        run(
            OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_HIDEREADONLY | OFN_NOCHANGEDIR,
            "另存为 Markdown",
            Some(suggested),
        )
    }
}
