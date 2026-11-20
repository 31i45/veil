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

use veil_ui::i18n::{t, tf, Key};
use veil_ui::ImageData;

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
        Err(tf(Key::ErrHresult, &[what, &format!("{:08X}", hr as u32)]))
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
        return Err(tf(
            Key::ErrHresult,
            &[t(Key::WicComInit), &format!("{:08X}", hr as u32)],
        ));
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
        t(Key::WicCreateFactory),
    )?;
    if factory.0.is_null() {
        return Err(t(Key::WicFactoryNull).into());
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
        t(Key::WicOpenDecoder),
    )?;
    if decoder.0.is_null() {
        return Err(t(Key::WicDecoderNull).into());
    }

    // IWICBitmapDecoder::GetFrame（vtable 13：QueryCapability/Initialize/GetContainerFormat/
    // GetDecoderInfo/CopyPalette/GetMetadataQueryReader/GetPreview/GetColorContexts/
    // GetThumbnail/GetFrameCount 之后）：取第 0 帧
    let mut frame = Com::empty();
    type GetFrameFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    let get_frame: GetFrameFn = method(decoder.0, 13);
    hresult(get_frame(decoder.0, 0, frame.out()), t(Key::WicReadFrame))?;
    if frame.0.is_null() {
        return Err(t(Key::WicFrameNull).into());
    }

    // IWICImagingFactory::CreateFormatConverter（vtable 10）
    let mut converter = Com::empty();
    type CreateConverterFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    let create_conv: CreateConverterFn = method(factory.0, 10);
    hresult(
        create_conv(factory.0, converter.out()),
        t(Key::WicCreateConverter),
    )?;
    if converter.0.is_null() {
        return Err(t(Key::WicConverterNull).into());
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
        t(Key::WicConvertPixels),
    )?;

    // IWICBitmapSource::GetSize（vtable 3）
    let (mut w, mut h) = (0u32, 0u32);
    type GetSizeFn = unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32;
    let get_size: GetSizeFn = method(converter.0, 3);
    hresult(get_size(converter.0, &mut w, &mut h), t(Key::WicReadSize))?;
    let pixels = w as u64 * h as u64;
    if w == 0 || h == 0 {
        return Err(tf(
            Key::ErrImageSizeInvalid,
            &[&w.to_string(), &h.to_string()],
        ));
    }
    if pixels > MAX_PIXELS {
        return Err(tf(Key::ErrImageTooLarge, &[&w.to_string(), &h.to_string()]));
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
        t(Key::WicReadPixels),
    )?;

    Ok(ImageData {
        pixels: buf,
        width: w,
        height: h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 每个用例独占一个临时目录（避免并行测试互相覆盖），按指定文件名落盘。
    /// WIC 按扩展名选解码器，合法图片用例必须用真实扩展名（如 `.bmp`）。
    fn temp_input_named(tag: &str, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veil_wic_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// 失败用例用无意义扩展名（内容是垃圾，靠解码器拒绝）。
    fn temp_input(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        temp_input_named(tag, "input.bin", bytes)
    }

    /// 垃圾字节：解码器拒绝并返回可读 Err（含归因 + HRESULT 诊断），不得 panic。
    #[test]
    fn garbage_bytes_yield_readable_error() {
        let path = temp_input("garbage", b"\x89PNG\r\n\x1a\nthis is not an image payload");
        let Err(err) = decode_image(&path) else {
            panic!("垃圾字节必须解码失败");
        };
        assert!(err.contains("打开图片解码器"), "错误须归因到解码器: {err}");
        assert!(err.contains("HRESULT"), "须保留系统诊断: {err}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// 空文件：与垃圾字节同路径失败，错误文本可读、不 panic。
    #[test]
    fn empty_file_yields_readable_error() {
        let path = temp_input("empty", b"");
        let Err(err) = decode_image(&path) else {
            panic!("空文件必须解码失败");
        };
        assert!(err.contains("打开图片解码器"), "错误须归因到解码器: {err}");
        assert!(err.contains("HRESULT"), "须保留系统诊断: {err}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// 不存在的路径：返回可读 Err（而不是静默成功或 panic）。
    #[test]
    fn missing_path_yields_readable_error() {
        let path = std::env::temp_dir().join(format!(
            "veil_wic_missing_{}_{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        assert!(!path.exists(), "夹具路径必须不存在");
        let Err(err) = decode_image(&path) else {
            panic!("不存在的路径必须失败");
        };
        assert!(err.contains("打开图片解码器"), "错误须归因到解码器: {err}");
        assert!(err.contains("HRESULT"), "须保留系统诊断: {err}");
    }

    // —— 成功路径：合法 BMP 必须真的解出像素（此前 3 个用例全为失败路径） ——

    /// 构造 24bpp 未压缩 BMP（BI_RGB）。`pixels_topdown` 为自顶向下的 BGR 序列，
    /// 长度须等于 width*height。行按 4 字节对齐；positive height = 自底向上（BMP 标准），
    /// 故文件内按行倒序写入。
    fn bmp_24(width: u32, height: u32, pixels_topdown: &[[u8; 3]]) -> Vec<u8> {
        assert_eq!(pixels_topdown.len(), (width * height) as usize);
        let row = (width * 3).div_ceil(4) * 4;
        let image = row * height;
        let mut out = Vec::with_capacity((54 + image) as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + image).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        // BITMAPINFOHEADER
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&image.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        // 自底向上写行：文件首行是图像最后一行
        for y in (0..height as usize).rev() {
            for x in 0..width as usize {
                out.extend_from_slice(&pixels_topdown[y * width as usize + x]);
            }
            out.resize(out.len() + (row - width * 3) as usize, 0);
        }
        out
    }

    /// 1x1 纯红 BMP：验证 COM 解码链真的产出 RGBA，而不只是"不报错"。
    #[test]
    fn decodes_minimal_one_by_one_bmp() {
        // 纯红：BGR = [0, 0, 255]
        let path = temp_input_named("bmp1", "input.bmp", &bmp_24(1, 1, &[[0, 0, 255]]));
        let img = decode_image(&path).expect("合法 BMP 必须解码成功");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.pixels.len(), 4, "1x1 RGBA8 = 4 字节");
        assert_eq!(img.pixels[0], 255, "BGR→RGBA：红通道应还原为 255");
        assert_eq!(img.pixels[1], 0);
        assert_eq!(img.pixels[2], 0);
        assert_eq!(img.pixels[3], 255, "无 alpha 的 BMP 应补为不透明");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// 2x2 四色 BMP：每行 6 字节按 4 对齐补到 8 字节。
    /// 用四种不同颜色，才能同时验证自底向上行序、行内像素顺序与填充不串色。
    #[test]
    fn decodes_two_by_two_bmp_with_row_padding() {
        // 自顶向下：红、绿、蓝、白（BGR 存储）
        let pixels = [
            [0u8, 0, 255],   // 左上 红
            [0, 255, 0],     // 右上 绿
            [255, 0, 0],     // 左下 蓝
            [255, 255, 255], // 右下 白
        ];
        let path = temp_input_named("bmp2", "input.bmp", &bmp_24(2, 2, &pixels));
        let img = decode_image(&path).expect("2x2 BMP 必须解码成功");
        assert_eq!((img.width, img.height), (2, 2));
        let expected = [
            255u8, 0, 0, 255, // 红
            0, 255, 0, 255, // 绿
            0, 0, 255, 255, // 蓝
            255, 255, 255, 255, // 白
        ];
        assert_eq!(
            img.pixels.as_slice(),
            &expected[..],
            "四色须按原行序/列序还原（自底向上翻转、行填充均不得串色）"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
