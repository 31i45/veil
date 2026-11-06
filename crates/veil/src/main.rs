//! veil 产品层入口（切片 6 公式渲染 latex-rust 位图，ADR-002；切片 7 HTML 导出 pulldown-cmark + 内嵌 SVG）。
//! 发布构建不带控制台窗口（GUI 子系统）；debug 构建保留控制台以便读诊断输出。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![allow(clippy::chunks_exact_to_as_chunks)] // 通道展开用 chunks_exact(4/3/2) 更直观

mod export;
mod platform;

use std::path::{Path, PathBuf};

use veil_ui::{decode, Doc, FileStore, ImageData, MathImage};

/// 无 CLI 参数时的示例文档，覆盖全部渲染路径：标题/粗体/斜体/代码/链接/行内公式/展示公式/代码块/中文。
const SAMPLE: &str = "# Veil 切片 8\n\n支持 **粗体**、*斜体*、`行内代码`、[链接](https://example.com) 与行内公式 $x^2 + y^2 = z^2$（光标移出渲染为图、移入显示源码）。\n\n块级公式（同规则，渲染图居中）：\n\n$$\n\\frac{a + b}{\\sqrt{c^2 + 1}} = \\sum_{k=0}^{n} k^2\n$$\n\n```\nfn main() { /* 代码块等宽 */ }\n```\n\n中文混排与 IME 输入：请用中文输入法在此段尾部打字验证候选框跟随。\n\n本页为无路径草稿：可拖入 .md 文件打开；关联文件后可直接拖入 PNG/JPEG 等图片插入正文（Ctrl+S 保存与 Ctrl+E 导出 HTML 仅对关联文件生效）。\n";

fn main() {
    let doc = match std::env::args().nth(1) {
        Some(p) => {
            let path = PathBuf::from(p);
            match Fs.load(&path).and_then(|b| decode(&b)) {
                Ok((text, bom)) => Doc {
                    text,
                    path: Some(path),
                    bom,
                    resume: false,
                },
                Err(e) => {
                    eprintln!("无法读取 {}: {e}", path.display());
                    std::process::exit(1);
                }
            }
        }
        None => Doc {
            text: SAMPLE.to_owned(),
            path: None,
            bom: false,
            resume: true,
        },
    };
    if let Err(e) = veil_ui::run(
        doc,
        veil_ui::Ports {
            clipboard: clipboard(),
            store: Box::new(Fs),
            math: math_renderer(),
            images: image_loader(),
            assets: asset_store(),
            exporter: exporter(),
            dialogs: platform::dialogs(),
            watcher: platform::watcher(),
            theme_env: platform::theme_env(),
            persist_env: platform::persist_env(),
            print: platform::print(),
            confirmer: platform::confirmer(),
            url_opener: platform::url_opener(),
        },
    ) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// HTML 导出器：latex-rust 出 SVG（与编辑器共用字体）；落盘复用 Fs 原子写。
/// 字体初始化失败时导出器拒绝工作（eprintln 可读错误），不影响编辑器其余功能。
fn exporter() -> Box<dyn veil_ui::Exporter> {
    match latex_rust::MathFont::stix_two_math() {
        Ok(font) => Box::new(export::HtmlExporter {
            font,
            store: Box::new(Fs),
        }),
        Err(e) => {
            eprintln!("数学字体初始化失败，HTML 导出不可用: {e}");
            Box::new(NoExport)
        }
    }
}

/// 字体初始化失败的兜底导出器：恒 Err（同 NoMath 模式）。
struct NoExport;
impl veil_ui::Exporter for NoExport {
    fn export_html(
        &mut self,
        _s: &veil_core::Session,
        _style: veil_ui::ExportStyle,
        _p: Option<&Path>,
    ) -> Result<PathBuf, String> {
        Err("数学字体不可用".into())
    }
}

/// latex-rust 渲染器（ADR-002）：STIX Two Math 字体内嵌于 crate，无 vendor 资产。
/// 手写 parse→layout→render_png 链以取 MathBox.height 算基线（行内公式与文本基线对齐）。
/// 3.0.1 的 render_png 文档仍明示忽略 PngOptions.display（display 只对 latex_to_png 生效），
/// 且基线本就需要 tree.height，故手写链保留。
/// 字号：块级 display 12pt；行内 text 8pt（8pt@144dpi = 16px em，与正文 BASE_FONT_SIZE 对齐）。
/// 颜色由调用方给定（切片 17）：深色主题下公式与正文同色，否则深底黑字不可读。
struct LatexMath {
    font: latex_rust::MathFont,
}
impl veil_ui::MathRenderer for LatexMath {
    fn render(
        &mut self,
        tex: &str,
        display: bool,
        scale: f32,
        color: [u8; 3],
    ) -> Result<MathImage, String> {
        let pt = if display { 12.0 } else { 8.0 };
        let dpi = 144.0 * scale;
        let ast = latex_rust::parse(tex).map_err(|e| e.to_string())?;
        let style = if display {
            latex_rust::MathStyle::Display
        } else {
            latex_rust::MathStyle::Text
        };
        let em_pt = latex_rust::Dim::parse(&format!("{pt}"));
        let tree = latex_rust::layout_with_em_size_pt(&ast, &self.font, style, &em_pt)
            .map_err(|e| e.to_string())?;
        let mut opt = latex_rust::PngOptions::new();
        opt.font_size_pt = latex_rust::Dim::parse(&format!("{pt}"));
        opt.dpi = latex_rust::Dim::parse(&format!("{dpi}"));
        opt.color = latex_rust::Color::rgb(color[0], color[1], color[2]);
        let png = latex_rust::render_png(&tree, &self.font, &opt).map_err(|e| e.to_string())?;
        let data = decode_png_rgba(&png)?;
        // 基线 = height(em) × em_px，与 raster.rs 的图内基线公式一致（baseline = height*em_px）
        let em_px = pt * dpi / 72.0;
        let h = f32::from_bits(tree.height.to_ieee32_bits());
        let baseline = (h * em_px).round().clamp(0.0, data.height as f32) as u32;
        Ok(MathImage {
            pixels: data.pixels,
            width: data.width,
            height: data.height,
            baseline,
        })
    }
}

fn math_renderer() -> Box<dyn veil_ui::MathRenderer> {
    match latex_rust::MathFont::stix_two_math() {
        Ok(font) => Box::new(LatexMath { font }),
        Err(e) => {
            eprintln!("数学字体初始化失败，公式将显示源码: {e}");
            Box::new(NoMath)
        }
    }
}

/// 字体初始化失败的兜底：恒 Err → 全部数学块降级源码显示（ADR-002 降级策略）。
struct NoMath;
impl veil_ui::MathRenderer for NoMath {
    fn render(
        &mut self,
        _tex: &str,
        _display: bool,
        _scale: f32,
        _color: [u8; 3],
    ) -> Result<MathImage, String> {
        Err("数学字体不可用".into())
    }
}

/// PNG 字节 → RGBA8 位图（切片 13 通用解码：latex-rust 产物为 RGBA8；文档图片可能是
/// RGB/灰度/调色板，统一经 png 变换展开为 straight alpha RGBA8）。
fn decode_png_rgba(png: &[u8]) -> Result<ImageData, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // 调色板/低位深灰度展开、16-bit 降为 8-bit、tRNS 转 alpha
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("PNG 输出尺寸溢出")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("非预期 PNG 位深: {:?}", info.bit_depth));
    }
    buf.truncate(info.buffer_size());
    let n = info.width as usize * info.height as usize;
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(n * 4);
            for c in buf.chunks_exact(3) {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(n * 4);
            for &g in &buf {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(n * 4);
            for c in buf.chunks_exact(2) {
                out.extend_from_slice(&[c[0], c[0], c[0], c[1]]);
            }
            out
        }
        // EXPAND 后不应出现调色板类型
        png::ColorType::Indexed => return Err("索引色 PNG 未展开".into()),
    };
    Ok(ImageData {
        pixels,
        width: info.width,
        height: info.height,
    })
}

/// 文档内图片加载器（切片 13 + 阶段 1）：std 读文件；PNG 走 png 解码，
/// 其余位图格式经 WIC。失败返回 Err 由视图降级 alt 文本（不每帧重试）。
struct FsImages;
impl veil_ui::ImageLoader for FsImages {
    fn load(&mut self, path: &Path) -> Result<ImageData, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        // PNG 走既有 png 解码（调色板/灰度/16-bit 经 png 变换展开）；
        // 其余格式（JPEG/WebP/BMP/GIF/TIFF）经 WIC，系统缺编解码器时返回可读错误
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            decode_png_rgba(&bytes)
        } else {
            platform::decode_image(path)
        }
    }
}

fn image_loader() -> Box<dyn veil_ui::ImageLoader> {
    Box::new(FsImages)
}

/// 粘贴图片资产存储（切片 14）：在文档旁 `<stem>.assets/` 落盘 PNG。
/// 文件名固定 `image-<n>.png`（n 递增、跳过已存在，不覆盖既有资产）；目录名由文档
/// stem sanitize 后拼 `.assets`（非法字符/分隔符替换为 `_`，禁止 `.`/`..` 目录穿越）。
struct FsAssets {
    next: u32,
}
impl veil_ui::AssetStore for FsAssets {
    fn save_pasted_image(&mut self, doc: &Path, image: &ImageData) -> Result<String, String> {
        let dir_name = asset_dir_name(doc);
        let dir = doc
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(&dir_name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{e}（创建 {}）", dir.display()))?;
        let (name, n) = unique_asset_name(&dir, self.next);
        self.next = n;
        let png = encode_png_rgba(image)?;
        let path = dir.join(&name);
        // 原子写：同目录 tmp + rename（同 Fs::save 语义）
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, png)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("{e}（写入 {}）", path.display()))?;
        Ok(format!("{dir_name}/{name}"))
    }
}

fn asset_store() -> Box<dyn veil_ui::AssetStore> {
    Box::new(FsAssets { next: 0 })
}

/// 资产目录名：`notes.md` → `notes.assets`（无扩展名按整名；空名 → `document.assets`）。
fn asset_dir_name(doc: &Path) -> String {
    let name = doc
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document");
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    format!("{}.assets", safe_stem(stem))
}

/// 文件名 sanitize：仅保留字母数字（含中文）、`-`、`_`，其余替换为 `_`；限长 64 字符。
/// 结果非空且不含 `.`，故不可能构成 `.`/`..` 目录项。只作用于目录名，天然无路径分隔符。
fn safe_stem(stem: &str) -> String {
    let s: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if s.is_empty() {
        "document".into()
    } else {
        s
    }
}

/// 资产文件名递增避让：从 `next`（0 视作 1）起找第一个不存在的 `image-<n>.png`。
fn unique_asset_name(dir: &Path, next: u32) -> (String, u32) {
    let mut n = next.max(1);
    loop {
        let name = format!("image-{n}.png");
        if !dir.join(&name).exists() {
            return (name, n);
        }
        n += 1;
    }
}

/// RGBA8 → PNG 编码（粘贴资产写入；尺寸与像素长度先校验，不信任外部输入）。
fn encode_png_rgba(image: &ImageData) -> Result<Vec<u8>, String> {
    let expect = image.width as usize * image.height as usize * 4;
    if image.pixels.len() != expect {
        return Err(format!(
            "位图尺寸不匹配: {} 字节 ≠ {}×{}×4",
            image.pixels.len(),
            image.width,
            image.height
        ));
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, image.width, image.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&image.pixels)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 文件系统实现：保存为原子写（同目录 tmp + rename），任何失败不留半截文件
/// （INSTANCE §9：Defender 受控文件夹拦截时给可读错误而非崩溃）。
struct Fs;
impl FileStore for Fs {
    fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| e.to_string())
    }
    fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".veil-tmp");
        let tmp = PathBuf::from(tmp);
        // std::fs::rename 在 Windows 覆盖既有目标（MoveFileEx REPLACE_EXISTING），同目录 rename 原子
        std::fs::write(&tmp, data)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| format!("{e}（写入 {}）", path.display()))
    }
    fn list_files(&mut self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                out.push(entry.path());
            }
        }
        Ok(out)
    }
}

/// 系统剪贴板（arboard 属产品层依赖，veil-ui 仅见 Clipboard trait，INSTANCE §6）。
fn clipboard() -> Box<dyn veil_ui::Clipboard> {
    struct Sys(arboard::Clipboard);
    impl veil_ui::Clipboard for Sys {
        fn get(&mut self) -> Option<String> {
            self.0.get_text().ok()
        }
        fn set(&mut self, text: &str) {
            let _ = self.0.set_text(text);
        }
        /// 富文本读取（切片 11）：系统剪贴板无 HTML 时 Err → 粘贴回退纯文本。
        fn get_html(&mut self) -> Option<String> {
            self.0.get().html().ok()
        }
        /// 位图读取（切片 14）：arboard 统一给出 RGBA8；尺寸/字节数不符视为无图。
        fn get_image(&mut self) -> Option<ImageData> {
            let img = self.0.get_image().ok()?;
            let expect = img.width.saturating_mul(img.height).saturating_mul(4);
            if img.bytes.len() != expect {
                return None;
            }
            Some(ImageData {
                pixels: img.bytes.into_owned(),
                width: img.width as u32,
                height: img.height as u32,
            })
        }
        /// 文件列表读取（资源管理器 Ctrl+C）：Windows 走 CF_HDROP；无文件列表时为空。
        fn get_files(&mut self) -> Vec<std::path::PathBuf> {
            self.0.get().file_list().unwrap_or_default()
        }
    }
    struct Noop; // 剪贴板初始化失败（如无显示服务器）：Ctrl+C/X/V 静默无效
    impl veil_ui::Clipboard for Noop {
        fn get(&mut self) -> Option<String> {
            None
        }
        fn set(&mut self, _text: &str) {}
    }
    match arboard::Clipboard::new() {
        Ok(c) => Box::new(Sys(c)),
        Err(e) => {
            eprintln!("剪贴板不可用: {e}");
            Box::new(Noop)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veil_ui::{AssetStore, ImageLoader};

    /// 真 latex-rust 渲染链（切片 8）：基线公式 baseline = height(em) × em_px，
    /// 与 crate 内 raster.rs 的图内基线一致。数值为 latex-rust 2.1.1 实测校准，
    /// 3.0.1 升级后复测不变（430 测试全绿，2026-10-25）：
    /// 行内 "x"（8pt@144dpi，em=16px）→ 9x8 图，"x" 无下伸部（depth=0）故 baseline == 图高。
    #[test]
    fn inline_render_has_baseline_and_pixels() {
        let mut m = math_renderer();
        let img = m
            .render("x", false, 1.0, [30, 30, 30])
            .expect("行内 x 渲染");
        assert_eq!(
            (img.width, img.height),
            (9, 8),
            "校准依据 latex-rust 2.1.1 实测"
        );
        assert_eq!(img.baseline, img.height, "无下伸字形：基线 = 图底");
        assert!(img.pixels.chunks_exact(4).any(|p| p[3] > 0), "像素非空");
        // 含下伸的公式（\frac 有 depth）：基线严格在图内
        let frac = m
            .render(r"\frac{1}{2}", false, 1.0, [30, 30, 30])
            .expect("行内 frac 渲染");
        assert!(
            frac.baseline > 0 && frac.baseline < frac.height,
            "有下伸：基线在图内"
        );
        let disp = m
            .render("x", true, 1.0, [30, 30, 30])
            .expect("display x 渲染");
        assert!(
            disp.height > img.height,
            "display 12pt 图高 > 行内 8pt 图高"
        );
    }

    #[test]
    fn invalid_tex_errs_for_source_fallback() {
        let mut m = math_renderer();
        assert!(
            m.render("{", false, 1.0, [30, 30, 30]).is_err(),
            "非法 tex 须 Err（视图降级源码显示）"
        );
    }

    /// 切片 17：公式字形颜色来自主题（深色主题白字/浅色主题黑字）。
    #[test]
    fn render_uses_requested_foreground_color() {
        let mut m = math_renderer();
        let white = m
            .render("x", false, 1.0, [255, 255, 255])
            .expect("白色行内渲染");
        let px = white
            .pixels
            .chunks_exact(4)
            .max_by_key(|p| p[3])
            .expect("图非空");
        assert_eq!(px[3], 255, "实心笔画存在全覆盖像素");
        assert_eq!(&px[..3], &[255, 255, 255], "浅色前景传入渲染器");
        let black = m.render("x", false, 1.0, [0, 0, 0]).expect("黑色行内渲染");
        let px = black
            .pixels
            .chunks_exact(4)
            .max_by_key(|p| p[3])
            .expect("图非空");
        assert_eq!(&px[..3], &[0, 0, 0], "深色前景传入渲染器");
    }

    /// 最小 PNG 生成器（任意 color type / 8-bit）。
    fn png_bytes(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn png_rgba_passthrough() {
        let img = decode_png_rgba(&png_bytes(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4])).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.pixels, vec![1, 2, 3, 4]);
    }

    #[test]
    fn png_rgb_and_grayscale_expand_to_rgba() {
        // RGB → 补 alpha 255
        let rgb = decode_png_rgba(&png_bytes(
            2,
            1,
            png::ColorType::Rgb,
            &[10, 20, 30, 40, 50, 60],
        ))
        .unwrap();
        assert_eq!(rgb.pixels, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        // 灰度 → 三通道复制 + alpha 255
        let gray = decode_png_rgba(&png_bytes(2, 1, png::ColorType::Grayscale, &[7, 8])).unwrap();
        assert_eq!(gray.pixels, vec![7, 7, 7, 255, 8, 8, 8, 255]);
        // 灰度+alpha → RGBA
        let ga =
            decode_png_rgba(&png_bytes(1, 1, png::ColorType::GrayscaleAlpha, &[9, 128])).unwrap();
        assert_eq!(ga.pixels, vec![9, 9, 9, 128]);
    }

    #[test]
    fn image_loader_reads_png_from_disk() {
        let path = std::env::temp_dir().join(format!("veil-img-{}.png", std::process::id()));
        std::fs::write(
            &path,
            png_bytes(1, 1, png::ColorType::Rgba, &[255, 0, 0, 255]),
        )
        .unwrap();
        let got = FsImages.load(&path);
        std::fs::remove_file(&path).ok();
        let img = got.expect("磁盘 PNG 应加载成功");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn image_loader_rejects_non_png_bytes() {
        assert!(
            decode_png_rgba(b"not a png").is_err(),
            "非 PNG → Err（视图降级 alt）"
        );
    }

    /// 阶段 1：JPEG 经 WIC 解码（系统内建编解码器）——1×1 红像素夹具。
    #[cfg(windows)]
    #[test]
    fn image_loader_decodes_jpeg_via_wic() {
        let path = std::env::temp_dir().join(format!("veil-img-{}.jpg", std::process::id()));
        std::fs::write(&path, include_bytes!("../tests/fixtures/red.jpg")).unwrap();
        let got = FsImages.load(&path);
        std::fs::remove_file(&path).ok();
        let img = got.expect("JPEG 应经 WIC 解码");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.pixels.len(), 4, "RGBA8 单像素");
        assert!(img.pixels[0] > 200, "红色通道保留: {:?}", img.pixels);
        assert!(img.pixels[3] > 0, "alpha 不透明");
    }

    /// 阶段 1：损坏的非 PNG 文件返回可读错误而非 panic（视图降级 alt）。
    #[cfg(windows)]
    #[test]
    fn image_loader_reports_wic_failure() {
        let path = std::env::temp_dir().join(format!("veil-bad-{}.jpg", std::process::id()));
        std::fs::write(&path, b"not an image").unwrap();
        let got = FsImages.load(&path);
        std::fs::remove_file(&path).ok();
        assert!(got.is_err(), "垃圾字节应解码失败");
    }

    // ---- 图片粘贴资产（切片 14）：safe_stem 防穿越、目录命名、递增避让、PNG 往返 ----

    #[test]
    fn asset_dir_name_and_stem_are_sanitized() {
        assert_eq!(asset_dir_name(Path::new("dir/notes.md")), "notes.assets");
        assert_eq!(
            asset_dir_name(Path::new("dir/无标题 文档.md")),
            "无标题_文档.assets"
        );
        assert_eq!(asset_dir_name(Path::new("..")), "document.assets");
        assert_eq!(
            safe_stem("../../etc"),
            "______etc",
            "分隔符/点号替换，杜绝目录穿越"
        );
        assert_eq!(safe_stem("a b"), "a_b");
        assert_eq!(safe_stem(""), "document");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("veil-assets-{}-{tag}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn asset_store_writes_unique_pngs_next_to_doc() {
        let dir = temp_dir("save");
        let doc = dir.join("notes.md");
        let img = ImageData {
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 8, 7, 6],
            width: 2,
            height: 2,
        };
        let mut s = FsAssets { next: 0 };
        assert_eq!(
            s.save_pasted_image(&doc, &img).unwrap(),
            "notes.assets/image-1.png"
        );
        assert_eq!(
            s.save_pasted_image(&doc, &img).unwrap(),
            "notes.assets/image-2.png"
        );
        // 新会话（next 归零）不得覆盖既有资产：跳过 image-1/2 取 image-3
        let mut s2 = FsAssets { next: 0 };
        assert_eq!(
            s2.save_pasted_image(&doc, &img).unwrap(),
            "notes.assets/image-3.png"
        );
        // 落盘确为可解码 PNG，且像素/RGBA 无损
        let bytes = std::fs::read(dir.join("notes.assets/image-1.png")).unwrap();
        let back = decode_png_rgba(&bytes).unwrap();
        assert_eq!((back.width, back.height), (2, 2));
        assert_eq!(back.pixels, img.pixels);
        // 无半截 tmp 残留
        assert!(!dir.join("notes.assets/.image-1.png.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encode_png_rejects_inconsistent_pixels() {
        let bad = ImageData {
            pixels: vec![0; 3],
            width: 2,
            height: 2,
        };
        assert!(encode_png_rgba(&bad).is_err(), "尺寸不符须拒绝，不写坏文件");
    }
}
