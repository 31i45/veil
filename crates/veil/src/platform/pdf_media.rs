//! PDF 媒体解码（Wave 4）：本地图片（WIC）与行内/块级公式（MathRenderer）→ RGB 位图。
//!
//! 约定：
//! - 只读本地文件；`http(s)` 图片不下载，由版面层降级为 alt 文本；
//! - 直通 alpha 的 RGBA 一律合成到白底（PDF 的 DeviceRGB 无 alpha 通道）；
//! - 单张 > 12 MP 按最近邻等比缩小，避免内存与 PDF 体积失控。

use std::path::{Path, PathBuf};

use veil_ui::i18n::{t, Key};
use veil_ui::MathRenderer;

use crate::platform::pdf::ImageData;
use crate::platform::pdf_layout::MediaProvider;

/// 图片按 96 dpi 解释（1 px = 0.75 pt）。
const IMAGE_PX_TO_PT: f32 = 0.75;
const MAX_MEGAPIXELS: u64 = 12_000_000;

pub struct PdfMedia {
    base_dir: Option<PathBuf>,
    math: Option<Box<dyn MathRenderer>>,
}

impl PdfMedia {
    pub fn new(base_dir: Option<PathBuf>, math: Option<Box<dyn MathRenderer>>) -> Self {
        Self { base_dir, math }
    }
}

impl MediaProvider for PdfMedia {
    fn image(&mut self, path: &Path) -> Result<ImageData, String> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir
                .as_ref()
                .ok_or_else(|| t(Key::ErrPdfNoDocPath).to_string())?
                .join(path)
        };
        let img = crate::platform::wic::decode_image(&abs)?;
        if img.width == 0 || img.height == 0 {
            return Err(t(Key::ErrPdfImageEmpty).into());
        }
        let (w, h, pixels) = clamp_megapixels(img.width, img.height, &img.pixels);
        let rgb = rgba_to_rgb(&pixels);
        Ok(ImageData {
            w,
            h,
            rgb,
            baseline_px: h as f32,
            px_to_pt: IMAGE_PX_TO_PT,
        })
    }

    fn math(&mut self, tex: &str, display: bool) -> Result<ImageData, String> {
        let renderer = self
            .math
            .as_mut()
            .ok_or_else(|| t(Key::ErrMathRendererUnavailable).to_string())?;
        // 行内 11pt / 块级 13pt：MathRenderer 以 8pt(text) / 12pt(display) 为基准，
        // 故 scale 取目标/基准；输出 dpi = 144 × scale，px→pt = 1/(2×scale)。
        let scale = if display { 13.0 / 12.0 } else { 11.0 / 8.0 };
        // 渲染器内部若 panic（TeX 极端输入），隔离为 Err 走源码降级，不穿透导出。
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.render(tex, display, scale, [24, 28, 35])
        }))
        .map_err(|_| t(Key::ErrMathRendererInternal).to_string())?;
        let img = rendered?;
        if img.width == 0 || img.height == 0 {
            return Err(t(Key::ErrMathRenderEmpty).into());
        }
        // 超长公式可能产出超大位图：复用 12MP 钳制，并按缩放比修正 px→pt 以保持物理尺寸。
        let (w, h, pixels) = clamp_megapixels(img.width, img.height, &img.pixels);
        let ratio = img.width as f32 / w as f32;
        let rgb = rgba_to_rgb(&pixels);
        Ok(ImageData {
            w,
            h,
            rgb,
            baseline_px: img.baseline as f32 / ratio,
            px_to_pt: ratio / (2.0 * scale),
        })
    }
}

/// 直通 alpha RGBA → 白底 RGB。
fn rgba_to_rgb(px: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(px.len() / 4 * 3);
    for c in px.chunks_exact(4) {
        let a = u32::from(c[3]);
        for ch in &c[..3] {
            out.push(((u32::from(*ch) * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    out
}

/// 超过 12 MP 时按最近邻等比缩小（返回新的宽高与 RGBA 数据）。
fn clamp_megapixels(w: u32, h: u32, px: &[u8]) -> (u32, u32, Vec<u8>) {
    let n = u64::from(w) * u64::from(h);
    if n <= MAX_MEGAPIXELS || w == 0 || h == 0 {
        return (w, h, px.to_vec());
    }
    let k = (MAX_MEGAPIXELS as f64 / n as f64).sqrt();
    let nw = ((f64::from(w) * k) as u32).max(1);
    let nh = ((f64::from(h) * k) as u32).max(1);
    let mut out = Vec::with_capacity((nw as usize) * (nh as usize) * 4);
    for y in 0..nh {
        let sy = (u64::from(y) * u64::from(h) / u64::from(nh)) as u32;
        for x in 0..nw {
            let sx = (u64::from(x) * u64::from(w) / u64::from(nw)) as u32;
            let si = ((u64::from(sy) * u64::from(w) + u64::from(sx)) * 4) as usize;
            if si + 4 <= px.len() {
                out.extend_from_slice(&px[si..si + 4]);
            } else {
                out.extend_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
    (nw, nh, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_composites_over_white() {
        let px = vec![0u8, 0, 0, 255, 0, 0, 0, 0];
        let rgb = rgba_to_rgb(&px);
        assert_eq!(&rgb[0..3], &[0, 0, 0]);
        assert_eq!(&rgb[3..6], &[255, 255, 255]);
    }

    #[test]
    fn megapixel_clamp_keeps_aspect() {
        let (w, h) = (6000u32, 4000u32);
        let px = vec![255u8; (w as usize) * (h as usize) * 4];
        let (nw, nh, out) = clamp_megapixels(w, h, &px);
        assert!(u64::from(nw) * u64::from(nh) <= MAX_MEGAPIXELS + 100_000);
        assert!((f64::from(nw) / f64::from(nh) - 1.5).abs() < 0.01);
        assert_eq!(out.len(), (nw as usize) * (nh as usize) * 4);
    }
}
