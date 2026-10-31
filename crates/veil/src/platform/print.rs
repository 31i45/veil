//! Win32 GDI 打印（3b，手写 FFI，零新增依赖）。
//! PRINTDLGW 取打印机 DC → 按打印机 DPI 全量重排分页（veil-egui 分页引擎）→
//! 逐页 SetDIBitsToDevice 贴帧（帧像素 XRGB little-endian = 32bpp BI_RGB 顶向下 DIB，零转换）。
//! PDF 导出走系统「Microsoft Print to PDF」虚拟打印机，同一码路。
//! 页边距固定 1 英寸（取 LOGPIXELSX，在对话框返回的可打印区内再留边）。

#![allow(non_snake_case)]

use std::ffi::c_void;

use veil_egui::{Print, PrintSession};

type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hglobal = *mut c_void;

/// PRINTDLGW（commdlg.h；repr(C) 自然布局，x64 下 size = 120）。
#[repr(C)]
struct PrintDlgW {
    l_struct_size: u32,
    hwnd_owner: Hwnd,
    h_dev_mode: Hglobal,
    h_dev_names: Hglobal,
    h_dc: Hdc,
    flags: u32,
    n_from_page: u16,
    n_to_page: u16,
    n_min_page: u16,
    n_max_page: u16,
    n_copies: u16,
    h_instance: *mut c_void,
    l_cust_data: isize,
    lpfn_print_hook: *mut c_void,
    lpfn_setup_hook: *mut c_void,
    lp_print_template_name: *const u16,
    lp_setup_template_name: *const u16,
    h_print_template: Hglobal,
    h_setup_template: Hglobal,
}

/// DOCINFOW（wingdi.h；x64 下 size = 40）。
#[repr(C)]
struct DocInfoW {
    cb_size: i32,
    lpsz_doc_name: *const u16,
    lpsz_output: *const u16,
    lpsz_datatype: *const u16,
    fw_type: u32,
}

/// BITMAPINFOHEADER（wingdi.h；40 字节，BI_RGB 无调色板可直接当 BITMAPINFO 用）。
#[repr(C)]
struct BitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<PrintDlgW>() == 120);
    assert!(std::mem::size_of::<DocInfoW>() == 40);
    assert!(std::mem::size_of::<BitmapInfoHeader>() == 40);
};

#[link(name = "comdlg32")]
extern "system" {
    fn PrintDlgW(pd: *mut PrintDlgW) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetDeviceCaps(hdc: Hdc, index: i32) -> i32;
    fn StartDocW(hdc: Hdc, lpdi: *const DocInfoW) -> i32;
    fn EndDoc(hdc: Hdc) -> i32;
    fn AbortDoc(hdc: Hdc) -> i32;
    fn StartPage(hdc: Hdc) -> i32;
    fn EndPage(hdc: Hdc) -> i32;
    fn DeleteDC(hdc: Hdc) -> i32;
    fn SetDIBitsToDevice(
        hdc: Hdc,
        x_dest: i32,
        y_dest: i32,
        w: u32,
        h: u32,
        x_src: i32,
        y_src: i32,
        start_scan: u32,
        c_lines: u32,
        lpv_bits: *const c_void,
        lpbmi: *const BitmapInfoHeader,
        color_use: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalFree(mem: Hglobal) -> Hglobal;
}

#[link(name = "user32")]
extern "system" {
    fn GetActiveWindow() -> Hwnd;
}

const PD_ALLPAGES: u32 = 0x0000_0000;
const PD_PAGENUMS: u32 = 0x0000_0002;
const PD_NOSELECTION: u32 = 0x0000_0004;
const PD_RETURNDC: u32 = 0x0000_0100;

const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
const LOGPIXELSX: i32 = 88;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 打印作业体（begin 之后必经路径）：StartDoc → 逐页贴帧 → EndDoc/AbortDoc。
/// 返回 Ok(已输出页数) 或 Err(提示文案)。任何失败都不留半开作业。
fn run_print_job(
    hdc: Hdc,
    session: &mut PrintSession,
    from: usize,
    to: usize,
) -> Result<usize, &'static str> {
    let doc_name = wide("Veil 文档");
    let di = DocInfoW {
        cb_size: std::mem::size_of::<DocInfoW>() as i32,
        lpsz_doc_name: doc_name.as_ptr(),
        lpsz_output: std::ptr::null(),
        lpsz_datatype: std::ptr::null(),
        fw_type: 0,
    };
    if unsafe { StartDocW(hdc, &di) } <= 0 {
        return Err("打印失败：无法开始打印作业");
    }
    let mut done = 0;
    for i in (from - 1)..to {
        let page_ok = unsafe { StartPage(hdc) } > 0 && {
            let frame = session.render_page(i);
            let bmi = BitmapInfoHeader {
                bi_size: 40,
                bi_width: frame.width as i32,
                bi_height: -(frame.height as i32), // 负值 = 顶向下，与帧行序一致
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: 0, // BI_RGB
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            };
            let (w, h) = (frame.width, frame.height);
            let margin = unsafe { GetDeviceCaps(hdc, LOGPIXELSX) }.max(0);
            let lines = unsafe {
                SetDIBitsToDevice(
                    hdc,
                    margin,
                    margin,
                    w,
                    h,
                    0,
                    0,
                    0,
                    h,
                    frame.pixels.as_ptr() as *const c_void,
                    &bmi,
                    0, // DIB_RGB_COLORS
                )
            };
            lines == h as i32
        } && unsafe { EndPage(hdc) } > 0;
        if !page_ok {
            unsafe { AbortDoc(hdc) };
            return Err("打印中断：部分页面未输出");
        }
        done += 1;
    }
    unsafe { EndDoc(hdc) };
    Ok(done)
}

/// Win32 打印端口：系统打印对话框 + GDI 位图输出。
pub struct WinPrint;

impl Print for WinPrint {
    fn print(&mut self, session: &mut PrintSession) {
        let owner = session
            .window_handle()
            .map(|w| w.hwnd as Hwnd)
            .unwrap_or_else(|| unsafe { GetActiveWindow() });
        let mut pd = PrintDlgW {
            l_struct_size: std::mem::size_of::<PrintDlgW>() as u32,
            hwnd_owner: owner,
            h_dev_mode: std::ptr::null_mut(),
            h_dev_names: std::ptr::null_mut(),
            h_dc: std::ptr::null_mut(),
            flags: PD_ALLPAGES | PD_RETURNDC | PD_NOSELECTION,
            n_from_page: 1,
            n_to_page: 1,
            n_min_page: 1,
            n_max_page: u16::MAX,
            n_copies: 1,
            h_instance: std::ptr::null_mut(),
            l_cust_data: 0,
            lpfn_print_hook: std::ptr::null_mut(),
            lpfn_setup_hook: std::ptr::null_mut(),
            lp_print_template_name: std::ptr::null(),
            lp_setup_template_name: std::ptr::null(),
            h_print_template: std::ptr::null_mut(),
            h_setup_template: std::ptr::null_mut(),
        };
        if unsafe { PrintDlgW(&mut pd) } == 0 {
            return; // 用户取消：静默返回，不打断工作流
        }
        // 对话框分配的全局内存句柄按约释放（PD_RETURNDC 下 hDC 由我们 DeleteDC）
        if !pd.h_dev_mode.is_null() {
            unsafe { GlobalFree(pd.h_dev_mode) };
        }
        if !pd.h_dev_names.is_null() {
            unsafe { GlobalFree(pd.h_dev_names) };
        }
        let hdc = pd.h_dc;
        if hdc.is_null() {
            session.notice("打印失败：无法获取打印机上下文");
            return;
        }
        // 页帧 = 可打印区 - 两侧 1 英寸边距；排版缩放 = 打印机 DPI / 96
        let dpi = unsafe { GetDeviceCaps(hdc, LOGPIXELSX) }.max(1);
        let page_w = unsafe { GetDeviceCaps(hdc, HORZRES) };
        let page_h = unsafe { GetDeviceCaps(hdc, VERTRES) };
        let frame_w = (page_w - 2 * dpi).max(1) as u32;
        let frame_h = (page_h - 2 * dpi).max(1) as u32;
        let scale = dpi as f32 / 96.0;
        // begin 之后所有路径必须 end（会话契约）；页码范围此时才可知总页数并钳位
        let pages = session.begin(frame_w, frame_h, scale);
        let (from, to) = if pd.flags & PD_PAGENUMS != 0 {
            let f = (pd.n_from_page as usize).clamp(1, pages);
            (f, (pd.n_to_page as usize).clamp(f, pages))
        } else {
            (1, pages)
        };
        let result = run_print_job(hdc, session, from, to);
        unsafe { DeleteDC(hdc) };
        session.end();
        match result {
            Ok(n) => session.notice(&format!("已发送 {n} 页到打印机")),
            Err(e) => session.notice(e),
        }
    }
}
