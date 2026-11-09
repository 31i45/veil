//! Win32 文件对话框（手写 FFI，零新增依赖）。
//!
//! 打开：`GetOpenFileNameW`，沿用系统「上次位置」，符合用户预期。
//! 保存/导出：COM `IFileSaveDialog`。老 API `GetSaveFileNameW` 在 Win7+ 有
//! 「初始目录与首调用值相同时退回上次位置」的规则，实测无法稳定定位到文档目录；
//! `IFileSaveDialog::SetFolder` 是唯一确定性的定位方式（导出必须让用户看清写到哪）。

#![allow(non_snake_case)]

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use veil_ui::FileDialog;

use super::wide;

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

/// GUID（guiddef.h）：按 Windows 内存布局排列，字符串形式见各常量注释。
#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// COMDLG_FILTERSPEC（shobjidl.h）。
#[repr(C)]
struct FilterSpec {
    name: Lpcwstr,
    spec: Lpcwstr,
}

/// CLSID_FileSaveDialog {C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}
/// （注册表 `HKCR\CLSID` 中名为 "File Save Dialog" 的实现，由 comdlg32 提供）
const CLSID_FILE_SAVE_DIALOG: Guid = Guid {
    data1: 0xC0B4_E2F3,
    data2: 0xBA21,
    data3: 0x4773,
    data4: [0x8D, 0xBA, 0x33, 0x5E, 0xC9, 0x46, 0xEB, 0x8B],
};

/// IID_IFileSaveDialog {84BCCD23-5FDE-4CDB-AEA4-AF64B83D78AB}（shobjidl_core.h）
const IID_IFILE_SAVE_DIALOG: Guid = Guid {
    data1: 0x84BC_CD23,
    data2: 0x5FDE,
    data3: 0x4CDB,
    data4: [0xAE, 0xA4, 0xAF, 0x64, 0xB8, 0x3D, 0x78, 0xAB],
};

/// IID_IShellItem {43826D1E-E718-42EE-BC55-A1E261C37BFE}
const IID_ISHELL_ITEM: Guid = Guid {
    data1: 0x4382_6D1E,
    data2: 0xE718,
    data3: 0x42EE,
    data4: [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
};

/// IFileSaveDialog vtable 槽位：IUnknown 0..=2，IModalWindow 的 Show 独占 3，
/// IFileDialog 从 4 起按 shobjidl.h 声明顺序排列（IFileSaveDialog 从 27 起）。
const SLOT_SHOW: usize = 3;
const SLOT_SET_FILE_TYPES: usize = 4;
const SLOT_SET_OPTIONS: usize = 9;
const SLOT_GET_OPTIONS: usize = 10;
const SLOT_SET_FOLDER: usize = 12;
const SLOT_SET_FILE_NAME: usize = 15;
const SLOT_SET_TITLE: usize = 17;
const SLOT_GET_RESULT: usize = 20;
const SLOT_SET_DEFAULT_EXTENSION: usize = 22;
/// IShellItem 槽位：QueryInterface/AddRef/Release/BindToHandler/GetParent 之后的第 6 个。
const SLOT_ITEM_GET_DISPLAY_NAME: usize = 5;

type GetOptionsFn = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type SetOptionsFn = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type SetFileTypesFn = unsafe extern "system" fn(*mut c_void, u32, *const FilterSpec) -> i32;
type SetFolderFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type SetStrFn = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
type GetResultFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type ShowFn = unsafe extern "system" fn(*mut c_void, Hwnd) -> i32;
type GetDisplayNameFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(ofn: *mut OpenFileNameW) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetActiveWindow() -> Hwnd;
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pv: *mut c_void, coinit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        outer: *mut c_void,
        clsctx: u32,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32;
    fn CoTaskMemFree(pv: *mut c_void);
}

#[link(name = "shell32")]
extern "system" {
    fn SHCreateItemFromParsingName(
        path: *const u16,
        bind_ctx: *mut c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32;
}

const OFN_NOCHANGEDIR: u32 = 0x0000_0004;
const OFN_HIDEREADONLY: u32 = 0x0000_0008;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_EXPLORER: u32 = 0x0008_0000;

const COINIT_APARTMENTTHREADED: u32 = 0x2;
const CLSCTX_INPROC_SERVER: u32 = 0x1;

const FOS_OVERWRITEPROMPT: u32 = 0x0000_0002;
const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
const FOS_PATHMUSTEXIST: u32 = 0x0000_0800;

const SIGDN_FILESYSPATH: u32 = 0x8005_8000;

const MAX_PATH_U16: usize = 32_768;

/// 过滤器串：每对「描述\0模式\0」，整体以额外 \0 结尾。
fn filter() -> Vec<u16> {
    "Markdown (*.md;*.markdown)\0*.md;*.markdown\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect()
}

/// 图片过滤器（2f 插入图片）：与 image::is_image_path 同一族扩展名。
fn image_filter() -> Vec<u16> {
    "图片 (*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico)\0*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect()
}

/// 用户「文档」目录：被重定向或不存在时退回用户主目录。
/// 只用于预填对话框的初始位置，不参与任何写盘决策。
fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    let docs = PathBuf::from(&home).join("Documents");
    Some(if docs.is_dir() { docs } else { PathBuf::from(home) })
}

/// 拆分建议路径 → (文件名, 初始目录)。裸名（草稿首次保存/导出）落「文档」。
fn split_suggestion(suggested: &str) -> (String, Option<String>) {
    let p = PathBuf::from(suggested);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| suggested.to_string());
    let dir = match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(d) => Some(d.to_string_lossy().into_owned()),
        None => documents_dir().map(|d| d.to_string_lossy().into_owned()),
    };
    (name, dir)
}

/// 打开文件对话框（不预填文件名，定位交给系统「上次位置」）。
fn open_dialog(title: &str, filter: Vec<u16>, def_ext: &str) -> Option<PathBuf> {
    let title = wide(title);
    let def_ext = wide(def_ext);
    let mut buf = [0u16; MAX_PATH_U16];
    let mut ofn = OpenFileNameW {
        l_struct_size: std::mem::size_of::<OpenFileNameW>() as u32,
        hwnd_owner: unsafe { GetActiveWindow() },
        h_instance: std::ptr::null_mut(),
        lpstr_filter: filter.as_ptr(),
        lpstr_custom_filter: std::ptr::null_mut(),
        n_max_cust_filter: 0,
        n_filter_index: 1, // 1-based：默认第一个过滤器
        lpstr_file: buf.as_mut_ptr(),
        n_max_file: MAX_PATH_U16 as u32,
        lpstr_file_title: std::ptr::null_mut(),
        n_max_file_title: 0,
        lpstr_initial_dir: std::ptr::null(),
        lpstr_title: title.as_ptr(),
        flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR,
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
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

/// 取 COM 对象 vtable 第 `index` 个方法。
///
/// SAFETY: `obj` 必须是有效的 COM 接口指针；`index` 必须落在该接口 vtable 内，
/// 且槽位签名与 `F` 一致。所有调用点都用本文件内的常量槽位。
unsafe fn com_slot<F: Copy>(obj: *mut c_void, index: usize) -> F {
    let vtable = *(obj as *mut *const *const c_void);
    let slot = *vtable.add(index);
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const c_void>());
    std::mem::transmute_copy(&slot)
}

/// 释放一个 COM 引用（vtable 槽位 2 = Release）。
fn com_release(obj: *mut c_void) {
    if obj.is_null() {
        return;
    }
    let release: ReleaseFn = unsafe { com_slot(obj, 2) };
    unsafe { release(obj) };
}

/// IShellItem → 文件系统完整路径（SIGDN_FILESYSPATH）。返回值由 COM 分配，这里负责释放。
unsafe fn item_path(item: *mut c_void) -> Option<PathBuf> {
    let get_display: GetDisplayNameFn = com_slot(item, SLOT_ITEM_GET_DISPLAY_NAME);
    let mut raw: *mut u16 = std::ptr::null_mut();
    if get_display(item, SIGDN_FILESYSPATH, &mut raw) < 0 || raw.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *raw.add(len) != 0 {
        len += 1;
    }
    let path = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
    CoTaskMemFree(raw as *mut c_void);
    Some(path)
}

/// COM 保存对话框：`SetFolder` 保证初始目录就是 `dir`，不受系统「上次位置」影响。
/// 返回 None = 用户取消（或对话框不可用），调用方据此不产生输出。
fn com_save(
    title: &str,
    file_name: &str,
    dir: Option<&str>,
    filter_label: &str,
    filter_pattern: &str,
    def_ext: &str,
) -> Option<PathBuf> {
    // S_OK = 本次初始化由我们收尾；S_FALSE = 本线程已是 STA（无需收尾）。
    let hr = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let owns_com = hr == 0;
    let out = unsafe {
        com_save_inner(title, file_name, dir, filter_label, filter_pattern, def_ext)
    };
    if owns_com {
        unsafe { CoUninitialize() };
    }
    out
}

/// SAFETY: 必须在已初始化的线程上调用；参数都是 Rust 借用的 &str，生命周期覆盖整次调用。
unsafe fn com_save_inner(
    title: &str,
    file_name: &str,
    dir: Option<&str>,
    filter_label: &str,
    filter_pattern: &str,
    def_ext: &str,
) -> Option<PathBuf> {
    let mut dialog: *mut c_void = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &CLSID_FILE_SAVE_DIALOG,
        std::ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_IFILE_SAVE_DIALOG,
        &mut dialog,
    );
    if hr < 0 || dialog.is_null() {
        return None;
    }

    // 覆盖要由系统确认；只接受文件系统路径，且目录必须存在。
    let get_options: GetOptionsFn = com_slot(dialog, SLOT_GET_OPTIONS);
    let set_options: SetOptionsFn = com_slot(dialog, SLOT_SET_OPTIONS);
    let mut opts = 0u32;
    if get_options(dialog, &mut opts) >= 0 {
        set_options(
            dialog,
            opts | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
        );
    }

    // 过滤器：目标类型 + 所有文件（与打开对话框的观感一致）。
    let label = wide(filter_label);
    let pattern = wide(filter_pattern);
    let all_label = wide("所有文件 (*.*)");
    let all_pattern = wide("*.*");
    let specs = [
        FilterSpec {
            name: label.as_ptr(),
            spec: pattern.as_ptr(),
        },
        FilterSpec {
            name: all_label.as_ptr(),
            spec: all_pattern.as_ptr(),
        },
    ];
    let set_types: SetFileTypesFn = com_slot(dialog, SLOT_SET_FILE_TYPES);
    set_types(dialog, specs.len() as u32, specs.as_ptr());

    // 初始目录：SetFolder 优先级高于 MRU，这是本模块换用 COM 的唯一原因。
    if let Some(dir) = dir {
        let dir_w = wide(dir);
        let mut folder: *mut c_void = std::ptr::null_mut();
        if SHCreateItemFromParsingName(
            dir_w.as_ptr(),
            std::ptr::null_mut(),
            &IID_ISHELL_ITEM,
            &mut folder,
        ) >= 0
            && !folder.is_null()
        {
            let set_folder: SetFolderFn = com_slot(dialog, SLOT_SET_FOLDER);
            set_folder(dialog, folder);
            com_release(folder);
        }
    }

    let name_w = wide(file_name);
    let set_name: SetStrFn = com_slot(dialog, SLOT_SET_FILE_NAME);
    set_name(dialog, name_w.as_ptr());

    let title_w = wide(title);
    let set_title: SetStrFn = com_slot(dialog, SLOT_SET_TITLE);
    set_title(dialog, title_w.as_ptr());

    let ext_w = wide(def_ext);
    let set_ext: SetStrFn = com_slot(dialog, SLOT_SET_DEFAULT_EXTENSION);
    set_ext(dialog, ext_w.as_ptr());

    let show: ShowFn = com_slot(dialog, SLOT_SHOW);
    if show(dialog, GetActiveWindow()) != 0 {
        com_release(dialog);
        return None; // 取消（HRESULT_FROM_WIN32(ERROR_CANCELLED)）或错误
    }

    let get_result: GetResultFn = com_slot(dialog, SLOT_GET_RESULT);
    let mut item: *mut c_void = std::ptr::null_mut();
    let out = if get_result(dialog, &mut item) >= 0 && !item.is_null() {
        let path = item_path(item);
        com_release(item);
        path
    } else {
        None
    };
    com_release(dialog);
    out
}

pub struct WinDialogs;

impl FileDialog for WinDialogs {
    fn open_markdown(&mut self) -> Option<PathBuf> {
        open_dialog("打开 Markdown", filter(), "md")
    }

    fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf> {
        let (name, dir) = split_suggestion(suggested);
        com_save(
            "另存为 Markdown",
            &name,
            dir.as_deref(),
            "Markdown (*.md;*.markdown)",
            "*.md;*.markdown",
            "md",
        )
    }

    fn open_image(&mut self) -> Option<PathBuf> {
        open_dialog("插入图片", image_filter(), "png")
    }

    fn save_export(&mut self, suggested: &str, ext: &str) -> Option<PathBuf> {
        let (name, dir) = split_suggestion(suggested);
        let (title, label, pattern) = if ext.eq_ignore_ascii_case("pdf") {
            ("导出 PDF", "PDF (*.pdf)", "*.pdf")
        } else {
            ("导出 HTML", "HTML (*.html)", "*.html")
        };
        com_save(title, &name, dir.as_deref(), label, pattern, ext)
    }
}
