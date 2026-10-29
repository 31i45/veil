//! Windows Imaging Component（WIC）解码：JPEG/WebP/BMP/GIF/TIFF 等 → straight alpha RGBA8。
//!
//! 手写 COM FFI（零新增依赖，INSTANCE §6）：只用到 v1 接口的少数方法，vtable 下标
//! 与 wincodec.h 声明顺序一一对应；每个 HRESULT 都校验，任一失败按声明顺序释放对象，
//! 不把未校验的指针带出本模块。WIC 的格式支持由系统编解码器决定（JPEG/PNG/BMP/GIF/
//! TIFF 内建；WebP 需要系统已安装对应编解码器），缺编解码器时返回可读错误而非崩溃。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use veil_egui::ImageData;

/// COM GUID（16 字节，repr(C) 与 Windows ABI 一致）。
#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const CLSID_WIC_IMAGING_FACTORY: Guid = Guid {
    data1: 0xCACAF262,
    data2: 0x9370,
    data3: 0x4615,
    data4: [0xA1, 0x3B, 0x9F, 0x55, 0x39, 0xDA, 0x4C, 0x0A],
};
const IID_IWIC_IMAGING_FACTORY: Guid = Guid {
    data1: 0xEC5EC8A9,
    data2: 0xC395,
    data3: 0x4314,
    data4: [0x9C, 0x77, 0x54, 0xD7, 0xA9, 0x35, 0xFF, 0x70],
};
const GUID_WICPIXELFORMAT_32BPP_RGBA: Guid = Guid {
    data1: 0xF5C7AD2D,
    data2: 0x6A8D,
    data3: 0x43DD,
    data4: [0xA7, 0xA8, 0xA2, 0x99, 0x35, 0x26, 0x1A, 0xE9],
};

const CLSCTX_INPROC_SERVER: u32 = 0x1;
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
const GENERIC_READ: u32 = 0x8000_0000;
/// WICDecodeOptions：按需解析元数据，避免为解码像素额外读取全部元数据块。
const WIC_DECODE_METADATA_CACHE_ON_DEMAND: u32 = 0;
/// WICBitmapDitherTypeNone / WICBitmapPaletteTypeCustom。
const WIC_DITHER_NONE: u32 = 0;
const WIC_PALETTE_CUSTOM: u32 = 0;
/// 解码结果上限：1 亿像素（400MB RGBA）。超出视为异常输入，不分配。
const MAX_PIXELS: u64 = 100_000_000;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        outer: *mut c_void,
        cls_context: u32,
        riid: *const Guid,
        out: *mut *mut c_void,
    ) -> i32;
}

/// IWICBitmapSource::CopyPixels 的矩形参数：null = 整张图（本实现只取整图）。
type RectPtr = *const c_void;

/// COM 对象所有权包装：离开作用域即 Release（空指针安全）。
struct Com(*mut c_void);

impl Com {
    fn empty() -> Self {
        Com(ptr::null_mut())
    }

    fn out(&mut self) -> *mut *mut c_void {
        &mut self.0
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
        unsafe {
            let vt = *(self.0 as *const *const usize);
            let release: ReleaseFn = std::mem::transmute(*vt.add(2));
            release(self.0);
        }
    }
}

/// 取 vtable 第 `index` 个方法的函数指针（调用方按 wincodec.h 声明顺序给类型）。
unsafe fn method<T: Copy>(obj: *mut c_void, index: usize) -> T {
    let vt = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vt.add(index))
}

fn hresult(hr: i32, what: &str) -> Result<(), String> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(format!("{what}失败（HRESULT 0x{:08X}）", hr as u32))
    }
}

/// COM 初始化守卫：成功（含 S_FALSE 已初始化）即配对 CoUninitialize。
struct ComGuard {
    initialized: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

/// 按路径解码任意 WIC 支持的图片格式为 RGBA8。
pub fn decode_image(path: &Path) -> Result<ImageData, String> {
    unsafe { decode_impl(path) }
}

unsafe fn decode_impl(path: &Path) -> Result<ImageData, String> {
    let hr = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED);
    if hr < 0 && hr != RPC_E_CHANGED_MODE {
        return Err(format!("COM 初始化失败（HRESULT 0x{:08X}）", hr as u32));
    }
    // S_OK/S_FALSE 都需配对 CoUninitialize；RPC_E_CHANGED_MODE 说明线程已在
    // 其它套间（winit 的 OLE 拖放），本次未初始化、不得反初始化
    let _com = ComGuard {
        initialized: hr >= 0,
    };

    let mut factory = Com::empty();
    hresult(
        CoCreateInstance(
            &CLSID_WIC_IMAGING_FACTORY,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IWIC_IMAGING_FACTORY,
            factory.out(),
        ),
        "创建 WIC 工厂",
    )?;
    if factory.0.is_null() {
        return Err("创建 WIC 工厂返回空指针".into());
    }

    // IWICImagingFactory::CreateDecoderFromFilename（vtable 3）
    let mut decoder = Com::empty();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    type CreateDecoderFn = unsafe extern "system" fn(
        *mut c_void,
        *const u16,
        *const Guid,
        u32,
        u32,
        *mut *mut c_void,
    ) -> i32;
    let create: CreateDecoderFn = method(factory.0, 3);
    hresult(
        create(
            factory.0,
            wide.as_ptr(),
            ptr::null(),
            GENERIC_READ,
            WIC_DECODE_METADATA_CACHE_ON_DEMAND,
            decoder.out(),
        ),
        "打开图片解码器",
    )?;
    if decoder.0.is_null() {
        return Err("图片解码器返回空指针".into());
    }

    // IWICBitmapDecoder::GetFrame（vtable 13：QueryCapability/Initialize/GetContainerFormat/
    // GetDecoderInfo/CopyPalette/GetMetadataQueryReader/GetPreview/GetColorContexts/
    // GetThumbnail/GetFrameCount 之后）：取第 0 帧
    let mut frame = Com::empty();
    type GetFrameFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    let get_frame: GetFrameFn = method(decoder.0, 13);
    hresult(get_frame(decoder.0, 0, frame.out()), "读取图片帧")?;
    if frame.0.is_null() {
        return Err("图片帧返回空指针".into());
    }

    // IWICImagingFactory::CreateFormatConverter（vtable 10）
    let mut converter = Com::empty();
    type CreateConverterFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let create_conv: CreateConverterFn = method(factory.0, 10);
    hresult(create_conv(factory.0, converter.out()), "创建格式转换器")?;
    if converter.0.is_null() {
        return Err("格式转换器返回空指针".into());
    }

    // IWICFormatConverter::Initialize（vtable 8：IWICBitmapSource 5 项之后）
    type InitializeFn = unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const Guid,
        u32,
        *mut c_void,
        f64,
        u32,
    ) -> i32;
    let init_conv: InitializeFn = method(converter.0, 8);
    hresult(
        init_conv(
            converter.0,
            frame.0,
            &GUID_WICPIXELFORMAT_32BPP_RGBA,
            WIC_DITHER_NONE,
            ptr::null_mut(),
            0.0,
            WIC_PALETTE_CUSTOM,
        ),
        "转换像素格式",
    )?;

    // IWICBitmapSource::GetSize（vtable 3）
    let (mut w, mut h) = (0u32, 0u32);
    type GetSizeFn = unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32;
    let get_size: GetSizeFn = method(converter.0, 3);
    hresult(get_size(converter.0, &mut w, &mut h), "读取图片尺寸")?;
    let pixels = w as u64 * h as u64;
    if w == 0 || h == 0 {
        return Err(format!("图片尺寸无效: {w}×{h}"));
    }
    if pixels > MAX_PIXELS {
        return Err(format!("图片过大: {w}×{h}（上限 1 亿像素）"));
    }

    // IWICBitmapSource::CopyPixels（vtable 7）：null 矩形 = 整张图
    let stride = w * 4;
    let mut buf = vec![0u8; stride as usize * h as usize];
    type CopyPixelsFn = unsafe extern "system" fn(*mut c_void, RectPtr, u32, u32, *mut u8) -> i32;
    let copy_pixels: CopyPixelsFn = method(converter.0, 7);
    hresult(
        copy_pixels(
            converter.0,
            ptr::null(),
            stride,
            buf.len() as u32,
            buf.as_mut_ptr(),
        ),
        "读取图片像素",
    )?;

    Ok(ImageData {
        pixels: buf,
        width: w,
        height: h,
    })
}
