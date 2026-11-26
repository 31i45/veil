//! Win32 GDI 打印（3b，手写 FFI，零新增依赖）。
//! PRINTDLGW 取打印机 DC → **与 PDF 导出同一 `pdf_layout` 版面**（分页/书签同源）→
//! 逐页把 DrawOp 画到打印机 DC（文字走 GDI，图片走 StretchDIBits）。
//! PDF 导出只走原生矢量管线（Type0/CIDFontType2），失败时报告可操作错误，不产生位图替代文件。
//! 打印版面不可用时回退旧的屏幕渲染打印（保证仍能出纸，并在状态栏明示降级）。

#![allow(non_snake_case)]

use std::ffi::c_void;

use veil_ui::i18n::{t, tf, Key};
use veil_ui::{path_label, Print, PrintSession};

use super::pdf::{
    find_cjk_font, DrawOp, FontMetrics, ImageData, TextMetrics, TextRun, A4_HEIGHT, A4_WIDTH,
};
use super::pdf_layout::{layout_document, DocLayout, LayoutOpts, PageGeom};
use super::pdf_media::PdfMedia;
use super::wide;

type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hglobal = *mut c_void;
type Hgdiobj = *mut c_void;
type Hbrush = *mut c_void;
type Hpen = *mut c_void;
type Hfont = *mut c_void;

#[repr(C)]
struct RectI32 {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PointI32 {
    x: i32,
    y: i32,
}

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

/// DEVMODEW（wingdi.h；x64 下 220 字节）。只用到 dmCopies/dmCollate，
/// 其余字段按标准布局保留以保证偏移正确（dmCopies 在偏移 86）。
#[repr(C)]
struct DevModeW {
    dm_device_name: [u16; 32],
    dm_spec_version: u16,
    dm_driver_version: u16,
    dm_size: u16,
    dm_driver_extra: u16,
    dm_fields: u32,
    dm_orientation: i16,
    dm_paper_size: i16,
    dm_paper_length: i16,
    dm_paper_width: i16,
    dm_scale: i16,
    dm_copies: i16,
    dm_default_source: i16,
    dm_print_quality: i16,
    dm_color: i16,
    dm_duplex: i16,
    dm_y_resolution: i16,
    dm_tt_option: i16,
    dm_collate: i16,
    dm_form_name: [u16; 32],
    dm_log_pixels: u16,
    dm_bits_per_pel: u32,
    dm_pels_width: u32,
    dm_pels_height: u32,
    dm_display_flags: u32,
    dm_display_frequency: u32,
    dm_icm_method: u32,
    dm_icm_intent: u32,
    dm_media_type: u32,
    dm_dither_type: u32,
    dm_reserved1: u32,
    dm_reserved2: u32,
    dm_panning_width: u32,
    dm_panning_height: u32,
}

/// DM_COPIES / DM_COLLATE（wingdi.h）：声明 DEVMODE 中哪些字段有效。
const DM_COPIES: u32 = 0x0000_0100;
const DM_COLLATE: u32 = 0x0000_1000;

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<PrintDlgW>() == 120);
    assert!(std::mem::size_of::<DocInfoW>() == 40);
    assert!(std::mem::size_of::<BitmapInfoHeader>() == 40);
    assert!(std::mem::size_of::<DevModeW>() == 220);
};

#[link(name = "comdlg32")]
extern "system" {
    fn PrintDlgW(pd: *mut PrintDlgW) -> i32;
    fn CommDlgExtendedError() -> u32;
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
    fn ResetDCW(hdc: Hdc, lpdm: *const c_void) -> Hdc;
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
    fn StretchDIBits(
        hdc: Hdc,
        x_dest: i32,
        y_dest: i32,
        dest_w: i32,
        dest_h: i32,
        x_src: i32,
        y_src: i32,
        src_w: i32,
        src_h: i32,
        lpv_bits: *const c_void,
        lpbmi: *const BitmapInfoHeader,
        color_use: u32,
        rop: u32,
    ) -> i32;
    fn SetBkMode(hdc: Hdc, mode: i32) -> i32;
    fn SetTextColor(hdc: Hdc, color: u32) -> u32;
    fn SetTextAlign(hdc: Hdc, align: u32) -> u32;
    fn TextOutW(hdc: Hdc, x: i32, y: i32, text: *const u16, len: i32) -> i32;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face_name: *const u16,
    ) -> Hfont;
    fn SelectObject(hdc: Hdc, obj: Hgdiobj) -> Hgdiobj;
    fn DeleteObject(obj: Hgdiobj) -> i32;
    fn CreateSolidBrush(color: u32) -> Hbrush;
    fn CreatePen(style: i32, width: i32, color: u32) -> Hpen;
    fn FillRect(hdc: Hdc, rect: *const RectI32, brush: Hbrush) -> i32;
    fn MoveToEx(hdc: Hdc, x: i32, y: i32, old: *mut PointI32) -> i32;
    fn LineTo(hdc: Hdc, x: i32, y: i32) -> i32;
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

// GDI 文本/图形常量（wingdi.h）
const TRANSPARENT: i32 = 1;
const TA_LEFT: u32 = 0;
const TA_BASELINE: u32 = 24;
const DEFAULT_CHARSET: u32 = 1;
const ANTIALIASED_QUALITY: u32 = 4;
const PS_SOLID: i32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;

/// Windows COLORREF：0x00BBGGRR。
fn rgb(color: [u8; 3]) -> u32 {
    color[0] as u32 | (color[1] as u32) << 8 | (color[2] as u32) << 16
}

/// 打印作业体（begin 之后必经路径）：StartDoc → 逐页贴帧 → EndDoc/AbortDoc。
/// `margin`：页帧在物理页内的贴图偏移（像素）。
/// 返回 Ok(已输出页数) 或 Err(提示文案)。任何失败都不留半开作业。
fn run_print_job(
    hdc: Hdc,
    session: &mut PrintSession,
    from: usize,
    to: usize,
    margin: i32,
) -> Result<usize, &'static str> {
    let doc_name = wide(t(Key::PrintDocumentName));
    let di = DocInfoW {
        cb_size: std::mem::size_of::<DocInfoW>() as i32,
        lpsz_doc_name: doc_name.as_ptr(),
        lpsz_output: std::ptr::null(),
        lpsz_datatype: std::ptr::null(),
        fw_type: 0,
    };
    if unsafe { StartDocW(hdc, &di) } <= 0 {
        return Err(t(Key::PrintErrStartJob));
    }
    let mut done = 0;
    for i in (from - 1)..to {
        let page_ok = unsafe { StartPage(hdc) } > 0
            && {
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
            }
            && unsafe { EndPage(hdc) } > 0;
        if !page_ok {
            unsafe { AbortDoc(hdc) };
            return Err(t(Key::PrintErrInterruptedPartial));
        }
        done += 1;
    }
    unsafe { EndDoc(hdc) };
    Ok(done)
}

/// 同目录临时目标：驱动写到全新文件，成功后替换正式目标。
/// 这样「覆盖已有 PDF」时即使打印失败也不会毁掉用户原文件（纯函数，便于单测）。
fn temp_pdf_path(target: &std::path::Path) -> std::path::PathBuf {
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    target.with_file_name(format!(".{stem}.{}.tmp.pdf", std::process::id()))
}

/// 用临时文件替换目标：旧文件先改名为备份（Windows 的 rename 不允许目标已存在），
/// 新文件落位成功后才删备份；任何一步失败都把旧文件恢复回去。
fn replace_file(tmp: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let backup = target.with_file_name(format!(".{stem}.{}.bak", std::process::id()));
    let had_old = target.exists();
    if had_old {
        std::fs::rename(target, &backup)
            .map_err(|e| tf(Key::PrintErrBackupOld, &[&e.to_string()]))?;
    }
    if let Err(e) = std::fs::rename(tmp, target) {
        if had_old {
            let _ = std::fs::rename(&backup, target); // 落位失败：把旧文件放回去
        }
        return Err(tf(Key::PrintErrWriteTarget, &[&e.to_string()]));
    }
    if had_old {
        let _ = std::fs::remove_file(&backup);
    }
    Ok(())
}

/// 打印用版面快照：与 PDF 导出共用 `pdf_layout`（含目录/页脚/图片/公式/多级书签）。
struct PrintLayout {
    layout: DocLayout,
    /// 度量与版面同源：绘制下划线/删除线时复用同一字宽，不再向 GDI 二次测量。
    metrics: FontMetrics,
    font_path: &'static str,
}

/// 构建与 PDF 导出完全同源的版面。失败时返回原因，由调用方决定回退策略。
fn build_print_layout(session: &mut PrintSession) -> Result<PrintLayout, String> {
    let font_path = find_cjk_font().ok_or_else(|| t(Key::PrintErrNoCjkFont).to_string())?;
    let markdown = session.markdown_text();
    if markdown.trim().is_empty() {
        return Err(t(Key::PrintErrDocEmpty).into());
    }
    let metrics = FontMetrics::load(std::path::Path::new(font_path), markdown.chars())?;
    let cfg = session.export_options();
    let geom = PageGeom::a4(session.export_margin_mm());
    let opts = LayoutOpts {
        toc: cfg.toc,
        page_numbers: cfg.page_numbers,
    };
    let mut media = PdfMedia::new(session.document_dir(), Some(crate::math_renderer()));
    let mermaids = session.mermaid_images();
    let layout = layout_document(
        session.session(),
        &metrics,
        geom,
        opts,
        Some(&mut media),
        &mermaids,
    );
    if layout.pages.is_empty() {
        return Err(t(Key::PrintErrNoPrintablePages).into());
    }
    Ok(PrintLayout {
        layout,
        metrics,
        font_path,
    })
}

/// 逐页把 DrawOp 画到打印机 DC：A4 版面等比缩放居中到可打印区。
/// 打印与 PDF 的页数/分页/图片布局因此逐页一致。
fn run_layout_job(
    hdc: Hdc,
    pl: &PrintLayout,
    page_w: i32,
    page_h: i32,
    from: usize,
    to: usize,
) -> Result<usize, &'static str> {
    let doc_name = wide(t(Key::PrintDocumentName));
    let di = DocInfoW {
        cb_size: std::mem::size_of::<DocInfoW>() as i32,
        lpsz_doc_name: doc_name.as_ptr(),
        lpsz_output: std::ptr::null(),
        lpsz_datatype: std::ptr::null(),
        fw_type: 0,
    };
    if unsafe { StartDocW(hdc, &di) } <= 0 {
        return Err(t(Key::PrintErrStartJob));
    }
    // 等比缩放 + 居中：不拉伸版面比例（与导出的 A4 观感一致）。
    let s = (page_w as f32 / A4_WIDTH)
        .min(page_h as f32 / A4_HEIGHT)
        .max(0.01);
    let off_x = (page_w as f32 - A4_WIDTH * s) * 0.5;
    let off_y = (page_h as f32 - A4_HEIGHT * s) * 0.5;

    let mut done = 0;
    for i in (from - 1)..to {
        let Some(ops) = pl.layout.pages.get(i) else {
            break;
        };
        if unsafe { StartPage(hdc) } <= 0 {
            unsafe { AbortDoc(hdc) };
            return Err(t(Key::PrintErrInterruptedNewPage));
        }
        for op in ops {
            draw_op(
                hdc,
                op,
                &pl.layout.images,
                &pl.metrics,
                pl.font_path,
                s,
                off_x,
                off_y,
            );
        }
        if unsafe { EndPage(hdc) } <= 0 {
            unsafe { AbortDoc(hdc) };
            return Err(t(Key::PrintErrInterruptedPartial));
        }
        done += 1;
    }
    if unsafe { EndDoc(hdc) } <= 0 {
        return Err(t(Key::PrintErrInterruptedEnd));
    }
    Ok(done)
}

/// 单个绘制指令 → GDI。坐标单位为 pt，乘 `s` 后加页内偏移。
#[allow(clippy::too_many_arguments)] // GDI 变换参数天然多
fn draw_op(
    hdc: Hdc,
    op: &DrawOp,
    images: &[ImageData],
    metrics: &FontMetrics,
    font_path: &str,
    s: f32,
    off_x: f32,
    off_y: f32,
) {
    match op {
        DrawOp::Text(run) => draw_text_run(hdc, run, metrics, font_path, s, off_x, off_y),
        DrawOp::Rect { x, y, w, h, color } => {
            let brush = unsafe { CreateSolidBrush(rgb(*color)) };
            let rect = rect_i32(*x, *y, *w, *h, s, off_x, off_y);
            unsafe {
                FillRect(hdc, &rect, brush);
                DeleteObject(brush);
            }
        }
        DrawOp::Line {
            x1,
            y1,
            x2,
            y2,
            width,
            color,
            ..
        } => {
            let pen =
                unsafe { CreatePen(PS_SOLID, (*width * s).round().max(1.0) as i32, rgb(*color)) };
            let old = unsafe { SelectObject(hdc, pen) };
            unsafe {
                MoveToEx(
                    hdc,
                    (off_x + *x1 * s).round() as i32,
                    (off_y + *y1 * s).round() as i32,
                    std::ptr::null_mut(),
                );
                LineTo(
                    hdc,
                    (off_x + *x2 * s).round() as i32,
                    (off_y + *y2 * s).round() as i32,
                );
                SelectObject(hdc, old);
                DeleteObject(pen);
            }
        }
        DrawOp::Image { x, y, w, h, id } => {
            let Some(img) = images.get(*id) else {
                return;
            };
            draw_image(hdc, img, *x, *y, *w, *h, s, off_x, off_y);
        }
    }
}

fn rect_i32(x: f32, y: f32, w: f32, h: f32, s: f32, off_x: f32, off_y: f32) -> RectI32 {
    RectI32 {
        left: (off_x + x * s).round() as i32,
        top: (off_y + y * s).round() as i32,
        right: (off_x + (x + w) * s).round() as i32,
        bottom: (off_y + (y + h) * s).round() as i32,
    }
}

/// 文本：GDI 字体按 pt 缩放，逐 TextRun 显式定位（与 PDF 同一坐标）。
fn draw_text_run(
    hdc: Hdc,
    run: &TextRun,
    metrics: &FontMetrics,
    font_path: &str,
    s: f32,
    off_x: f32,
    off_y: f32,
) {
    if run.text.is_empty() {
        return;
    }
    let height = -((run.size * s).round() as i32).max(1);
    let face = font_face_for(font_path);
    let font = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            if run.bold { 700 } else { 400 },
            run.italic as u32,
            0,
            0,
            DEFAULT_CHARSET,
            0,
            0,
            ANTIALIASED_QUALITY,
            0,
            face.as_ptr(),
        )
    };
    let old = unsafe { SelectObject(hdc, font) };
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextAlign(hdc, TA_LEFT | TA_BASELINE);
        SetTextColor(hdc, rgb(run.color));
    }
    let x = off_x + run.x * s;
    let y = off_y + run.y * s;
    let tw = metrics.text_width(&run.text, run.size) * s;
    if let Some(bg) = run.highlight {
        let brush = unsafe { CreateSolidBrush(rgb(bg)) };
        let rect = RectI32 {
            left: x.round() as i32,
            top: (y - metrics.ascent(run.size) * s).round() as i32,
            right: (x + tw).round() as i32,
            bottom: (y + (run.size - metrics.ascent(run.size)) * s).round() as i32,
        };
        unsafe {
            FillRect(hdc, &rect, brush);
            DeleteObject(brush);
        }
    }
    let units: Vec<u16> = run.text.encode_utf16().collect();
    unsafe {
        TextOutW(
            hdc,
            x.round() as i32,
            y.round() as i32,
            units.as_ptr(),
            units.len() as i32,
        );
    }
    // 下划线/删除线：与 PDF 同源按字宽画线（不依赖 GDI 字形装饰）。
    for (dy, on) in [
        ((run.size * 0.14).max(1.0), run.underline),
        ((-run.size * 0.30), run.strike),
    ] {
        if !on {
            continue;
        }
        let pen = unsafe {
            CreatePen(
                PS_SOLID,
                (run.size * 0.06 * s).round().max(1.0) as i32,
                rgb(run.color),
            )
        };
        let prev = unsafe { SelectObject(hdc, pen) };
        let ly = (y + dy * s).round() as i32;
        unsafe {
            MoveToEx(hdc, x.round() as i32, ly, std::ptr::null_mut());
            LineTo(hdc, (x + tw).round() as i32, ly);
            SelectObject(hdc, prev);
            DeleteObject(pen);
        }
    }
    unsafe {
        SelectObject(hdc, old);
        DeleteObject(font);
    }
}

/// PDF 字体链首项是 msyh.ttc；GDI 用同族名，保证打印与 PDF/预览同一字体观感。
fn font_face_for(font_path: &str) -> Vec<u16> {
    let name = if font_path.to_ascii_lowercase().contains("msyh") {
        "Microsoft YaHei"
    } else if font_path.to_ascii_lowercase().contains("deng") {
        "DengXian"
    } else if font_path.to_ascii_lowercase().contains("simhei") {
        "SimHei"
    } else {
        "SimSun"
    };
    wide(name)
}

/// 图片：RGB8 → 32bpp 顶向下 DIB，按版面矩形拉伸输出。
#[allow(clippy::too_many_arguments)]
fn draw_image(
    hdc: Hdc,
    img: &ImageData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    s: f32,
    off_x: f32,
    off_y: f32,
) {
    if img.w == 0 || img.h == 0 {
        return;
    }
    let mut bits = Vec::with_capacity(img.rgb.len() / 3 * 4);
    for px in img.rgb.chunks_exact(3) {
        bits.push(px[2]);
        bits.push(px[1]);
        bits.push(px[0]);
        bits.push(0);
    }
    let bmi = BitmapInfoHeader {
        bi_size: 40,
        bi_width: img.w as i32,
        bi_height: -(img.h as i32),
        bi_planes: 1,
        bi_bit_count: 32,
        bi_compression: 0,
        bi_size_image: 0,
        bi_x_pels_per_meter: 0,
        bi_y_pels_per_meter: 0,
        bi_clr_used: 0,
        bi_clr_important: 0,
    };
    let dest = rect_i32(x, y, w, h, s, off_x, off_y);
    unsafe {
        StretchDIBits(
            hdc,
            dest.left,
            dest.top,
            (dest.right - dest.left).max(1),
            (dest.bottom - dest.top).max(1),
            0,
            0,
            img.w as i32,
            img.h as i32,
            bits.as_ptr() as *const c_void,
            &bmi,
            0,
            SRCCOPY,
        );
    }
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
            // PrintDlgW 返回 0 既可能是用户取消也可能是 API 失败：CommDlgExtendedError
            // 为 0 表示用户取消（静默，不打断工作流）；非 0 为错误码，记录诊断。
            let err = unsafe { CommDlgExtendedError() };
            if err != 0 {
                eprintln!("打印对话框失败 (CommDlgExtendedError=0x{err:04x})");
            }
            return;
        }
        // 份数：对话框的 nCopies 写回 DEVMODE 并应用到 DC，驱动按份数/份内排序输出。
        // 不设 PD_USEDEVMODECOPIES，保证对话框里的份数输入权威。
        let copies = pd.n_copies.max(1);
        let hdc = pd.h_dc;
        if copies > 1 && !pd.h_dev_mode.is_null() {
            let dm = pd.h_dev_mode as *mut DevModeW;
            unsafe {
                (*dm).dm_fields |= DM_COPIES | DM_COLLATE;
                (*dm).dm_copies = copies.min(i16::MAX as u16) as i16;
                (*dm).dm_collate = 1;
                if !hdc.is_null() {
                    ResetDCW(hdc, pd.h_dev_mode);
                }
            }
        }
        // 对话框分配的全局内存句柄按约释放（PD_RETURNDC 下 hDC 由我们 DeleteDC）
        if !pd.h_dev_mode.is_null() {
            unsafe { GlobalFree(pd.h_dev_mode) };
        }
        if !pd.h_dev_names.is_null() {
            unsafe { GlobalFree(pd.h_dev_names) };
        }
        if hdc.is_null() {
            session.notice_error(t(Key::PrintNoPrinterContext));
            return;
        }
        // 用户已确认打印：补齐缺失的 Mermaid 渲染（同步、有预算），
        // 让 PDF/打印与预览同源；失败静默降级为源码围栏。
        session.prepare_mermaid();
        // 页帧 = 打印机可打印区（不再额外扣 1 英寸：A4 版面自带配置页边距）；
        // 版面与 PDF 导出共用 pdf_layout，打印只负责把同一页画到设备上。
        let dpi = unsafe { GetDeviceCaps(hdc, LOGPIXELSX) }.max(1);
        let page_w = unsafe { GetDeviceCaps(hdc, HORZRES) }.max(1);
        let page_h = unsafe { GetDeviceCaps(hdc, VERTRES) }.max(1);
        // 统一管线：先按 A4 版面排一次（与导出同源），再等比缩放到可打印区。
        match build_print_layout(session) {
            Ok(pl) => {
                let pages = pl.layout.pages.len();
                let (from, to) = if pd.flags & PD_PAGENUMS != 0 {
                    let f = (pd.n_from_page as usize).clamp(1, pages);
                    (f, (pd.n_to_page as usize).clamp(f, pages))
                } else {
                    (1, pages)
                };
                let result = run_layout_job(hdc, &pl, page_w, page_h, from, to);
                unsafe { DeleteDC(hdc) };
                match result {
                    Ok(n) => session.notice(&tf(Key::PrintSentPagesUnified, &[&n.to_string()])),
                    Err(e) => session.print_error(e),
                }
            }
            Err(e) => {
                // 版面不可用（无 CJK 字体/空文档等）：回退屏幕渲染打印，明确告知降级原因。
                session.notice(&tf(Key::PrintUnifiedFallback, &[&e.to_string()]));
                let frame_w = (page_w - 2 * dpi).max(1) as u32;
                let frame_h = (page_h - 2 * dpi).max(1) as u32;
                let scale = dpi as f32 / 96.0;
                let pages = session.begin(frame_w, frame_h, scale);
                let (from, to) = if pd.flags & PD_PAGENUMS != 0 {
                    let f = (pd.n_from_page as usize).clamp(1, pages);
                    (f, (pd.n_to_page as usize).clamp(f, pages))
                } else {
                    (1, pages)
                };
                let result = run_print_job(hdc, session, from, to, dpi);
                unsafe { DeleteDC(hdc) };
                session.end();
                match result {
                    Ok(n) => session.notice(&tf(Key::PrintSentPagesScreen, &[&n.to_string()])),
                    Err(e) => session.print_error(e),
                }
            }
        }
    }

    fn export_pdf(&mut self, session: &mut PrintSession) {
        // 导出是显式副作用：每次都先让用户确认输出位置（取消即中止，不写盘）。
        // 建议路径为文档同目录 `<stem>.pdf`，草稿为裸名（平台层补「文档」目录）。
        let suggested = session.export_suggestion();
        let target = match session.choose_export_path(&suggested, "pdf") {
            Some(t) => t,
            None => {
                session.notice(t(Key::PrintExportCancelled));
                return;
            }
        };
        // DEF-T09-004：重名导出递增命名（`name (2).pdf`），不替换既有文件。
        let target = crate::export::unique_file_path(&target);

        // 位置已确认：补齐缺失的 Mermaid 渲染（同步、有预算），保证导出一致性。
        session.prepare_mermaid();

        // 唯一出口：原生矢量 PDF（Type0/CIDFontType2 + ToUnicode，可搜索/可复制/带书签）。
        // 失败时 try_native_pdf 保证既有目标未被改动（tmp + 原子替换）；
        // 不弹打印对话框、不生成位图 PDF，只给出可操作错误。
        if let Err(e) = try_native_pdf(&target, session) {
            session.export_pdf_error(&e);
        }
    }
}

// =============================================================================
// 原生 PDF 集成（Wave 3A）
// =============================================================================

/// 尝试原生 PDF 导出。成功写盘 Ok(())，失败 Err(原因) 由调用方显示可操作错误。
fn try_native_pdf(target: &std::path::Path, session: &mut PrintSession) -> Result<(), String> {
    use super::pdf::{find_cjk_font, render_pages, FontMetrics};
    use super::pdf_layout::{layout_document, LayoutOpts, PageGeom};
    use super::pdf_media::PdfMedia;

    let font_path = find_cjk_font().ok_or_else(|| t(Key::PrintErrNoCjkFont).to_string())?;
    let font_path = std::path::Path::new(font_path);

    let markdown = session.markdown_text();
    if markdown.trim().is_empty() {
        return Err(t(Key::PrintErrDocEmpty).into());
    }

    // 分页与书签同源：版面引擎一次产出 pages 与 outline，禁止二次切分。
    let metrics = FontMetrics::load(font_path, markdown.chars())?;
    let cfg = session.export_options();
    let geom = PageGeom::a4(session.export_margin_mm());
    let opts = LayoutOpts {
        toc: cfg.toc,
        page_numbers: cfg.page_numbers,
    };
    let base_dir = session.document_dir();
    let mut media = PdfMedia::new(base_dir, Some(crate::math_renderer()));
    let mermaids = session.mermaid_images();

    let layout = layout_document(
        session.session(),
        &metrics,
        geom,
        opts,
        Some(&mut media),
        &mermaids,
    );

    if layout.pages.is_empty() {
        return Err(t(Key::PrintErrNoExportPages).into());
    }

    let bytes = render_pages(
        &layout.pages,
        &layout.images,
        &layout.outline,
        &layout.links,
        font_path,
    )?;

    if bytes.len() > 40 * 1024 * 1024 {
        return Err(tf(
            Key::PrintErrPdfTooLarge,
            &[&(bytes.len() / 1_000_000).to_string()],
        ));
    }

    let tmp = temp_pdf_path(target);
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(tf(Key::PrintErrWriteTemp, &[&e.to_string()]));
    }
    if let Err(e) = replace_file(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(tf(Key::PrintErrReplaceTarget, &[&e.to_string()]));
    }

    session.notice(&tf(
        Key::PrintExportedPdf,
        &[
            &path_label(Some(target)),
            &bytes.len().to_string(),
            &layout.pages.len().to_string(),
            &layout.outline.len().to_string(),
            &layout.images.len().to_string(),
        ],
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{font_face_for, rect_i32, replace_file, rgb, temp_pdf_path};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veil_pdf_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn temp_pdf_path_stays_next_to_target() {
        // 临时文件必须与目标同目录（跨卷 rename 会失败），且扩展名仍为 .pdf（驱动要求）
        let target = PathBuf::from("dir/doc.pdf");
        let tmp = temp_pdf_path(&target);
        assert_eq!(tmp.parent(), target.parent());
        let name = tmp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with(".doc."), "实际: {name}");
        assert!(name.ends_with(".tmp.pdf"), "实际: {name}");
    }

    #[test]
    fn replace_file_overwrites_existing_target() {
        // 覆盖已存在的 PDF：旧文件被替换，不产生 `doc (2).pdf` 之类的第二份文件
        let dir = temp_dir("replace");
        let target = dir.join("doc.pdf");
        let tmp = temp_pdf_path(&target);
        fs::write(&target, b"old").unwrap();
        fs::write(&tmp, b"new").unwrap();

        replace_file(&tmp, &target).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!tmp.exists(), "临时文件应被移走");
        let pdfs: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "pdf"))
            .collect();
        assert_eq!(pdfs.len(), 1, "目录中只应有一个 PDF");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_file_keeps_old_target_when_rename_fails() {
        // 临时文件不存在（模拟打印未产出）：替换失败，旧文件原样保留
        let dir = temp_dir("keep");
        let target = dir.join("doc.pdf");
        fs::write(&target, b"old").unwrap();

        assert!(replace_file(&dir.join("missing.tmp.pdf"), &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// B7：打印与 PDF 共用 DrawOp，颜色必须按 COLORREF（0x00BBGGRR）传给 GDI，
    /// 否则红蓝互换——中文/表格/代码再正确也会视觉失真。
    #[test]
    fn colorref_is_bgr_not_rgb() {
        assert_eq!(rgb([0x12, 0x34, 0x56]), 0x0056_3412);
        assert_eq!(rgb([0, 0, 0]), 0);
        assert_eq!(rgb([255, 255, 255]), 0x00FF_FFFF);
    }

    /// B7：A4 pt 版面等比缩放 + 居中映射到打印机可打印区。
    #[test]
    fn print_rect_scales_and_centers_layout() {
        let r = rect_i32(10.0, 20.0, 100.0, 50.0, 2.0, 5.0, 7.0);
        assert_eq!((r.left, r.top, r.right, r.bottom), (25, 47, 225, 147));
    }

    /// B7：GDI 字体族必须与 PDF 内嵌字体同族（msyh → Microsoft YaHei），
    /// 保证打印与导出/预览的中文观感一致。
    #[test]
    fn gdi_face_tracks_pdf_font_chain() {
        assert_eq!(
            wide_decode(&font_face_for("C:\\Windows\\Fonts\\msyh.ttc")),
            "Microsoft YaHei"
        );
        assert_eq!(
            wide_decode(&font_face_for("C:\\Windows\\Fonts\\Deng.ttf")),
            "DengXian"
        );
    }

    fn wide_decode(s: &[u16]) -> String {
        String::from_utf16_lossy(s)
            .trim_end_matches('\0')
            .to_string()
    }
}
