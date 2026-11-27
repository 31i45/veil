//! 原生 PDF 导出（Wave 4）：绘制原语 + Type0/CIDFontType2 + ToUnicode CMap + 多级书签。
//!
//! 本模块只负责「页面上的绘制指令 → PDF 字节」，版面由 `pdf_layout` 产出；
//! 两者分离让换行/分页可纯逻辑单测，不必依赖真实字体光栅。
//!
//! 依赖边界：pdf-writer / ttf-parser / subsetter 仅本模块使用。
//! 字体来源：运行时从 `C:\Windows\Fonts\` 读取 simhei.ttf → msyh.ttc → simsun.ttc 链。

#![allow(clippy::manual_find)]

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use pdf_writer::types::Predictor;
use pdf_writer::types::{ActionType, AnnotationFlags, AnnotationType, CidFontType, FontFlags};
use pdf_writer::{Filter, Name, Pdf, Rect, Ref, Str};

use super::pdf_layout::{LinkAnnot, LinkTarget};

use veil_ui::i18n::{t, tf, Key};

/// A4 默认尺寸（pt，72 pt / inch）。
pub const A4_WIDTH: f32 = 595.28;
pub const A4_HEIGHT: f32 = 841.89;

/// 字体链候选（Windows 字体，按优先级）。
/// 优先 msyh（微软雅黑）：与应用预览的中文 UI 字体一致，且拉丁字母为比例宽度；
/// simhei 仅作兜底——它的拉丁字母是半角等宽（全 500），会让 i/l 显空、W/m 显挤。
#[cfg(windows)]
const FONT_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑
    r"C:\Windows\Fonts\Deng.ttf",   // 等线
    r"C:\Windows\Fonts\simhei.ttf", // 黑体（兜底）
    r"C:\Windows\Fonts\simsun.ttc",
];

#[cfg(not(windows))]
const FONT_CANDIDATES: &[&str] = &[];

/// inline code 的真实等宽字体；找不到时保留主字体，不用比例字体伪装等宽。
#[cfg(windows)]
const MONOSPACE_FONT_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\consola.ttf",      // Consolas
    r"C:\Windows\Fonts\CascadiaMono.ttf", // Cascadia Mono
    r"C:\Windows\Fonts\CascadiaCode.ttf", // Cascadia Code
    r"C:\Windows\Fonts\cour.ttf",         // Courier New
];

#[cfg(not(windows))]
const MONOSPACE_FONT_CANDIDATES: &[&str] = &[];

/// 页面顶部起算的 Y → PDF 坐标系 Y（原点在左下角）。
pub fn y_from_top(top_pt: f32) -> f32 {
    A4_HEIGHT - top_pt
}

/// URI 文本 → PDF 字节串可安全承载的 ASCII：非 ASCII 字节按 UTF-8 百分号编码，
/// 避免查看器把 PDFDocEncoding 字节串误读成乱码链接。
fn uri_ascii(uri: &str) -> String {
    let mut out = String::with_capacity(uri.len());
    for b in uri.bytes() {
        if b.is_ascii() && !b.is_ascii_control() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 找一个可用的 Windows CJK 字体路径。
#[cfg(windows)]
pub fn find_cjk_font() -> Option<&'static str> {
    for p in FONT_CANDIDATES {
        if Path::new(p).exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(not(windows))]
pub fn find_cjk_font() -> Option<&'static str> {
    None
}

fn find_monospace_font() -> Option<&'static str> {
    MONOSPACE_FONT_CANDIDATES
        .iter()
        .copied()
        .find(|p| Path::new(p).exists())
}

/// 字符回退字体候选：CJK 主字体缺字（emoji / 非 BMP 符号 / CJK 扩展 B 等）时按序尝试。
///
/// 这里只代表「可能可用」；真正是否覆盖某码点必须由 `ttf_parser` 查字形，
/// 不能按文件名猜测——例如 Segoe UI Emoji 有 emoji 但无 `𠮷`（U+20BB7），
/// 而 SimSun-ExtB 恰恰只覆盖 CJK 扩展区。
#[cfg(windows)]
const FALLBACK_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\seguiemj.ttf",              // Segoe UI Emoji
    r"C:\Windows\Fonts\seguisym.ttf",              // Segoe UI Symbol
    r"C:\Windows\Fonts\simsunb.ttf",               // SimSun-ExtB（CJK 扩展 B/C/D）
    r"C:\Windows\Fonts\SimsunExtG.ttf",            // SimSun-ExtG（CJK 扩展 G）
    r"C:\Windows\Fonts\msyh.ttc",                  // 微软雅黑
    r"C:\Windows\Fonts\msyhbd.ttc",                // 微软雅黑 Bold
    r"C:\Windows\Fonts\simsun.ttc",                // 宋体
    r"C:\Windows\Fonts\NotoSansSC-VF.ttf",         // Noto Sans SC
    r"C:\Windows\Fonts\NotoSansCJK-Regular.ttc",   // Noto Sans CJK（部分系统安装）
    r"C:\Windows\Fonts\NotoSansCJKsc-Regular.otf", // Noto Sans CJK SC
];

/// 磁盘上真实存在的回退字体，按优先级排列。
/// 是否用于某个字符仍须经 `FontResource::covers` / `Face::glyph_index` 探测。
#[cfg(windows)]
pub fn fallback_font_candidates() -> Vec<&'static str> {
    FALLBACK_CANDIDATES
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
}

#[cfg(not(windows))]
pub fn fallback_font_candidates() -> Vec<&'static str> {
    Vec::new()
}

/// 首个可用回退字体（emoji 等通用场景）；逐字符选择请用 `fallback_font_candidates`。
#[cfg(all(test, windows))]
pub fn find_fallback_font() -> Option<&'static str> {
    fallback_font_candidates().into_iter().next()
}

#[cfg(all(test, not(windows)))]
pub fn find_fallback_font() -> Option<&'static str> {
    None
}

// =============================================================================
// 度量
// =============================================================================

/// 版面度量接口：换行与列宽只依赖这三个量，单测可注入假实现。
pub trait TextMetrics {
    /// 单字宽度（pt）。
    fn width_of(&self, c: char, size: f32) -> f32;

    /// 等宽字体实际覆盖该字符时返回其宽度；缺字由调用方走普通字体回退。
    fn width_of_monospace(&self, c: char, size: f32) -> Option<f32> {
        let _ = (c, size);
        None
    }

    fn width_of_style(&self, c: char, size: f32, monospace: bool) -> f32 {
        if monospace {
            if let Some(w) = self.width_of_monospace(c, size) {
                return w;
            }
        }
        self.width_of(c, size)
    }

    fn text_width(&self, s: &str, size: f32) -> f32 {
        s.chars().map(|c| self.width_of(c, size)).sum()
    }

    fn text_width_styled(&self, s: &str, size: f32, monospace: bool) -> f32 {
        s.chars()
            .map(|c| self.width_of_style(c, size, monospace))
            .sum()
    }

    /// 单行行高（含行距）。
    fn line_height(&self, size: f32) -> f32 {
        size * 1.55
    }

    /// 基线距行顶距离。
    fn ascent(&self, size: f32) -> f32 {
        size * 0.88
    }
}

/// 只读字体度量（不子集化）：版面阶段用，避免为测宽而子集化整篇字符。
pub struct FontMetrics {
    widths: HashMap<char, f32>,
    monospace_widths: Option<HashMap<char, f32>>,
    /// 缺字回退宽度（em 比例）。
    fallback: f32,
    pub ascent_ratio: f32,
}

impl FontMetrics {
    /// `chars` 应覆盖文档全部字符（含书签标题），未覆盖字符走回退宽度。
    pub fn load(font_path: &Path, chars: impl IntoIterator<Item = char>) -> Result<Self, String> {
        let bytes = std::fs::read(font_path).map_err(|e| {
            tf(
                Key::ErrFontRead,
                &[&font_path.display().to_string(), &e.to_string()],
            )
        })?;
        let face = ttf_parser::Face::parse(&bytes, 0)
            .map_err(|e| tf(Key::ErrFontParse, &[&e.to_string()]))?;
        let upem = face.units_per_em().max(1) as f32;
        let scale = 1000.0 / upem;

        let chars: BTreeSet<char> = chars.into_iter().collect();
        let mut widths = HashMap::new();
        let mut missing: Vec<char> = Vec::new();
        for c in chars.iter().copied() {
            if widths.contains_key(&c) {
                continue;
            }
            match face
                .glyph_index(c)
                .and_then(|gid| face.glyph_hor_advance(gid))
            {
                Some(a) => {
                    widths.insert(c, a as f32 * scale);
                }
                None => missing.push(c),
            }
        }
        // 主字体缺字（emoji / 非 BMP / CJK 扩展区）：逐个候选回退字体按真实
        // 字形覆盖测宽。不能按文件名假设覆盖范围，也不能把所有缺字记成同一宽度，
        // 否则换行位置与最终嵌入字体的步进不一致。
        if !missing.is_empty() {
            for path in fallback_font_candidates() {
                if missing.is_empty() {
                    break;
                }
                let Ok(bytes) = std::fs::read(path) else {
                    continue;
                };
                let Ok(fb) = ttf_parser::Face::parse(&bytes, 0) else {
                    continue;
                };
                let fb_scale = 1000.0 / fb.units_per_em().max(1) as f32;
                missing.retain(|&c| {
                    match fb.glyph_index(c).and_then(|gid| fb.glyph_hor_advance(gid)) {
                        Some(a) => {
                            widths.insert(c, a as f32 * fb_scale);
                            false
                        }
                        None => true,
                    }
                });
            }
        }

        let monospace_widths = find_monospace_font().and_then(|path| {
            let bytes = std::fs::read(path).ok()?;
            let mono = ttf_parser::Face::parse(&bytes, 0).ok()?;
            let mono_scale = 1000.0 / mono.units_per_em().max(1) as f32;
            let mut widths = HashMap::new();
            let mono_chars: BTreeSet<char> = chars.iter().copied().chain(' '..='~').collect();
            for c in mono_chars {
                if let Some(a) = mono
                    .glyph_index(c)
                    .and_then(|gid| mono.glyph_hor_advance(gid))
                {
                    widths.insert(c, a as f32 * mono_scale);
                }
            }
            Some(widths)
        });

        Ok(Self {
            widths,
            monospace_widths,
            fallback: 600.0,
            // YaHei 的 ascender 含大量上行留白（2167/2048≈1.06），直接使用会把基线压低；
            // 钳到 0.72–0.92 与常见排版基线一致。
            ascent_ratio: (face.ascender() as f32 * scale / 1000.0).clamp(0.72, 0.92),
        })
    }
}

impl TextMetrics for FontMetrics {
    fn width_of(&self, c: char, size: f32) -> f32 {
        let units = self.widths.get(&c).copied().unwrap_or(self.fallback);
        units * size / 1000.0
    }

    fn width_of_monospace(&self, c: char, size: f32) -> Option<f32> {
        self.monospace_widths
            .as_ref()?
            .get(&c)
            .map(|units| units * size / 1000.0)
    }

    fn ascent(&self, size: f32) -> f32 {
        self.ascent_ratio * size
    }
}

// =============================================================================
// 绘制原语
// =============================================================================

/// 已解码位图（RGB8，行主序）。`px_to_pt` 把像素换算为排版点数（图片按 96dpi，公式按渲染 dpi）。
#[derive(Clone)]
pub struct ImageData {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
    /// 图顶到基线的像素数（行内公式对齐用）。
    pub baseline_px: f32,
    pub px_to_pt: f32,
}

impl ImageData {
    pub fn pt_w(&self) -> f32 {
        self.w as f32 * self.px_to_pt
    }

    pub fn pt_h(&self) -> f32 {
        self.h as f32 * self.px_to_pt
    }

    pub fn baseline_pt(&self) -> f32 {
        self.baseline_px * self.px_to_pt
    }
}

/// 图像样本 Flate 压缩阈值：原始 RGB ≥ 64 KiB 才压缩，小图保持原始写入避免无谓 CPU。
const IMAGE_COMPRESS_MIN_BYTES: usize = 64 * 1024;

/// 拼接 PNG 中所有 IDAT 块的数据。PNG 的 IDAT 是单一 zlib 流的连续切片，
/// 拼接结果可直接交给 PDF `FlateDecode` 解码。
fn png_idat_stream(png: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.get(..8)? != SIGNATURE {
        return None;
    }
    let mut pos = 8;
    let mut zlib = Vec::new();
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data_end = pos.checked_add(8)?.checked_add(len)?;
        if data_end + 4 > png.len() {
            return None;
        }
        if kind == b"IDAT" {
            zlib.extend_from_slice(&png[pos + 8..data_end]);
        } else if kind == b"IEND" {
            break;
        }
        pos = data_end + 4;
    }
    (!zlib.is_empty()).then_some(zlib)
}

/// 把 RGB8 样本编码成 PNG（每行 NoFilter）并取出完整 zlib 流，供 PDF
/// `FlateDecode` + `Predictor 15` 使用。返回 None 表示无需压缩（样本过小）、
/// 通道数不符或编码失败，调用方应回退到原始写入。
fn compress_image_samples(rgb: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if rgb.len() < IMAGE_COMPRESS_MIN_BYTES {
        return None;
    }
    // 只接受严格的 RGB8 行主序样本，通道数不一致时保守回退。
    let expected = (w as usize).checked_mul(h as usize)?.checked_mul(3)?;
    if rgb.len() != expected {
        return None;
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        // NoFilter 让每行以字节 0 开头，正是 PDF Predictor 15 的 PNG 行格式。
        encoder.set_filter(png::Filter::NoFilter);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgb).ok()?;
        writer.finish().ok()?;
    }
    png_idat_stream(&png_bytes)
}

/// 写入一张 RGB8 位图 XObject：≥64 KiB 的大图压成 FlateDecode + PNG predictor，
/// 编码失败或小图回退原始写入，导出不因压缩失败而失败。
fn write_image_xobject(pdf: &mut Pdf, image_ref: Ref, img: &ImageData) {
    if let Some(samples) = compress_image_samples(&img.rgb, img.w, img.h) {
        let mut x = pdf.image_xobject(image_ref, &samples);
        x.width(img.w as i32)
            .height(img.h as i32)
            .color_space_name(Name(b"DeviceRGB"))
            .bits_per_component(8)
            .filter(Filter::FlateDecode);
        let mut parms = x.decode_parms();
        parms
            .predictor(Predictor::PngOptimum)
            .colors(3)
            .bits_per_component(8)
            .columns(img.w as i32);
    } else {
        let mut x = pdf.image_xobject(image_ref, &img.rgb);
        x.width(img.w as i32)
            .height(img.h as i32)
            .color_space_name(Name(b"DeviceRGB"))
            .bits_per_component(8);
    }
}

/// 一段同一样式的文本。`x`/`y` 为页面顶部起算，`y` 是基线。
#[derive(Clone, Debug)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    pub underline: bool,
    pub strike: bool,
    pub highlight: Option<[u8; 3]>,
    pub color: [u8; 3],
}

#[derive(Clone, Debug)]
pub enum DrawOp {
    Text(TextRun),
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 3],
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: [u8; 3],
    },
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        id: usize,
    },
}

/// 书签条目。`level` 为标题级别（1–6），用于构建父子树。
#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub level: u8,
    pub page_idx: usize,
    pub top_y: f32,
}

// =============================================================================
// ToUnicode CMap
// =============================================================================

/// 构建 ToUnicode CMap（CMapType 2，bfchar 格式）。
/// UTF-16 代理对（Emoji U+1F600+）用 4 字节 dest 编码。
fn build_to_unicode_cmap(cid_to_char: &[char]) -> Vec<u8> {
    let mut out = String::new();

    let mut bmp: Vec<(u16, u16)> = Vec::new();
    let mut astral: Vec<(u16, String)> = Vec::new();

    for (cid, &c) in cid_to_char.iter().enumerate() {
        if c == '\0' {
            continue;
        }
        let code = c as u32;
        if code <= 0xFFFF {
            bmp.push((cid as u16, code as u16));
        } else {
            let hi = 0xD800u32 + ((code - 0x10000) >> 10);
            let lo = 0xDC00u32 + ((code - 0x10000) & 0x3FF);
            astral.push((cid as u16, format!("{hi:04X}{lo:04X}")));
        }
    }

    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    out.push_str("/CMapName /Adobe-Identity-UCS def /CMapType 2 def\n");
    out.push_str("1 begincodespacerange <0000> <FFFF> endcodespacerange\n");

    // 规范限制：单个 beginbfchar 块最多 100 条映射。
    for chunk in bmp.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, code) in chunk {
            out.push_str(&format!("<{cid:04X}> <{code:04X}>\n"));
        }
        out.push_str("endbfchar\n");
    }

    for chunk in astral.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, hex_pair) in chunk {
            out.push_str(&format!("<{cid:04X}> <{hex_pair}>\n"));
        }
        out.push_str("endbfchar\n");
    }

    out.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    out.into_bytes()
}

// =============================================================================
// 字体资源
// =============================================================================

/// 字体资源：CID 映射 + 子集化字节 + 度量。
pub struct FontResource {
    pub char_to_cid: HashMap<char, u16>,
    pub cid_to_char: Vec<char>,
    pub subset_bytes: Vec<u8>,
    pub font_bbox: [f32; 4],
    pub ascent: f32,
    pub descent: f32,
    pub base_name: String,
    widths: HashMap<char, f32>,
}

impl FontResource {
    /// 从字体文件加载并子集化。`required_chars` = 所有页面与书签用到的字符。
    pub fn load_and_subset(
        font_path: &Path,
        required_chars: impl IntoIterator<Item = char>,
        no_subset: bool,
    ) -> Result<Self, String> {
        let font_bytes = std::fs::read(font_path).map_err(|e| {
            tf(
                Key::ErrFontRead,
                &[&font_path.display().to_string(), &e.to_string()],
            )
        })?;
        let face = ttf_parser::Face::parse(&font_bytes, 0)
            .map_err(|e| tf(Key::ErrFontParse, &[&e.to_string()]))?;

        let mut all_chars: BTreeSet<char> = BTreeSet::new();
        all_chars.insert(' ');
        for c in ' '..='~' {
            all_chars.insert(c);
        }
        for c in required_chars {
            all_chars.insert(c);
        }

        // 单一策略：subsetter remap + CID = remap 后编号 + Identity CIDToGIDMap
        let mut remapper = subsetter::GlyphRemapper::new();
        remapper.remap(0);
        let mut gid_to_cid: HashMap<u16, u16> = HashMap::new();
        gid_to_cid.insert(0, 0);
        let mut char_to_cid: HashMap<char, u16> = HashMap::new();
        char_to_cid.insert('\0', 0);

        for &c in &all_chars {
            if let Some(gid) = face.glyph_index(c) {
                let old = gid.0;
                let cid = if let Some(&existing) = gid_to_cid.get(&old) {
                    existing
                } else {
                    let new_cid = remapper.remap(old);
                    gid_to_cid.insert(old, new_cid);
                    new_cid
                };
                char_to_cid.insert(c, cid);
            }
        }

        let subset_bytes = if no_subset {
            font_bytes.clone()
        } else {
            subsetter::subset(&font_bytes, 0, &remapper)
                .map_err(|e| tf(Key::ErrFontSubset, &[&e.to_string()]))?
        };

        let max_cid = char_to_cid.values().copied().max().unwrap_or(0);
        let mut cid_to_char: Vec<char> = vec!['\0'];
        cid_to_char.resize(max_cid as usize + 1, '\0');
        for (&c, &cid) in &char_to_cid {
            let idx = cid as usize;
            if idx < cid_to_char.len() {
                cid_to_char[idx] = c;
            }
        }

        let units_per_em = face.units_per_em() as f32;
        let scale = 1000.0 / units_per_em;
        let bb = face.global_bounding_box();

        // 度量：CID 字体用同一份字符集
        let mut widths = HashMap::new();
        for &c in char_to_cid.keys() {
            if c == '\0' {
                continue;
            }
            let w = face
                .glyph_index(c)
                .and_then(|gid| face.glyph_hor_advance(gid))
                .map(|a| a as f32 * scale)
                .unwrap_or(600.0);
            widths.insert(c, w);
        }

        let base_name = font_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("CJKFont")
            .to_string();

        Ok(Self {
            char_to_cid,
            cid_to_char,
            subset_bytes,
            font_bbox: [
                bb.x_min as f32 * scale,
                bb.y_min as f32 * scale,
                bb.x_max as f32 * scale,
                bb.y_max as f32 * scale,
            ],
            ascent: face.ascender() as f32 * scale,
            descent: face.descender() as f32 * scale,
            base_name,
            widths,
        })
    }

    /// 把字符串编码成 Identity-H 的 hex CID 串 `<CID1CID2...>`。
    pub fn encode_cid_hex(&self, text: &str) -> String {
        let hex: String = text
            .chars()
            .map(|c| {
                let cid = self.cid_of(c);
                format!("{cid:04X}")
            })
            .collect();
        format!("<{hex}>")
    }

    /// 字体是否覆盖该字符（有可写出的 CID）。
    pub fn covers(&self, c: char) -> bool {
        self.char_to_cid.contains_key(&c)
    }

    /// 字符 → CID。字体无该字形时回退到 `?`（仍有 ToUnicode 与正确步进），
    /// 避免落到 CID 0/.notdef 造成「无字 + 宽度按 1em」的静默错位。
    pub fn cid_of(&self, c: char) -> u16 {
        if let Some(&cid) = self.char_to_cid.get(&c) {
            return cid;
        }
        self.char_to_cid.get(&'?').copied().unwrap_or(0)
    }

    /// 按 CID 顺序的宽度数组（1000 单位），供 CIDFont 的 `/W` 使用。
    pub fn widths_by_cid(&self) -> Vec<f32> {
        self.cid_to_char
            .iter()
            .map(|&c| {
                if c == '\0' {
                    1000.0
                } else {
                    self.widths.get(&c).copied().unwrap_or(1000.0)
                }
            })
            .collect()
    }
}

impl TextMetrics for FontResource {
    fn width_of(&self, c: char, size: f32) -> f32 {
        self.widths.get(&c).copied().unwrap_or(600.0) * size / 1000.0
    }
}

// =============================================================================
// PDF 渲染
// =============================================================================

fn color_cmd(c: [u8; 3], stroke: bool) -> String {
    let f = |v: u8| v as f32 / 255.0;
    format!(
        "{:.3} {:.3} {:.3} {}",
        f(c[0]),
        f(c[1]),
        f(c[2]),
        if stroke { "RG" } else { "rg" }
    )
}

/// 把一段文本按「主字体能否覆盖」切成若干 (是否用回退字体, 片段)。
/// 缺字且回退字体也无字形时仍留在主字体（走 `?` 兜底）。
#[cfg(test)]
fn split_font_runs(
    text: &str,
    primary: &FontResource,
    fallback: Option<&FontResource>,
) -> Vec<(bool, String)> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        let use_fb = !primary.covers(c) && fallback.is_some_and(|f| f.covers(c));
        match runs.last_mut() {
            Some((fb, s)) if *fb == use_fb => s.push(c),
            _ => runs.push((use_fb, c.to_string())),
        }
    }
    runs
}

/// 把一段文本按「第几个字体覆盖该字符」切成若干 (字体下标, 片段)。
/// 下标 0 = 主字体，1..=n = 回退字体（/F2../Fn）。所有字体都不覆盖时留在主字体。
fn split_font_runs_multi(text: &str, fonts: &[&FontResource]) -> Vec<(usize, String)> {
    let mut runs: Vec<(usize, String)> = Vec::new();
    for c in text.chars() {
        let idx = fonts.iter().position(|f| f.covers(c)).unwrap_or(0);
        match runs.last_mut() {
            Some((i, s)) if *i == idx => s.push(c),
            _ => runs.push((idx, c.to_string())),
        }
    }
    runs
}

/// 内容流（测试用两字体快捷入口）：主字体 + 可选单一回退字体。
#[cfg(test)]
fn content_stream(
    page: &[DrawOp],
    font: &FontResource,
    fallback: Option<&FontResource>,
) -> Vec<u8> {
    let fallbacks: Vec<&FontResource> = fallback.into_iter().collect();
    content_stream_with_fonts_and_mono(page, font, &fallbacks, None)
}

/// 内容流：文本按字体覆盖能力切 run，逐段切换 `/F1`、`/F2`…
/// 保证 emoji、CJK 扩展 B 等主字体缺字仍以真实字形与 ToUnicode 输出。
fn content_stream_with_fonts_and_mono(
    page: &[DrawOp],
    font: &FontResource,
    fallbacks: &[&FontResource],
    monospace: Option<&FontResource>,
) -> Vec<u8> {
    let mut out = String::new();
    for op in page {
        match op {
            DrawOp::Rect { x, y, w, h, color } => {
                out.push_str(&format!(
                    "{} {x:.2} {:.2} {w:.2} {h:.2} re f\n",
                    color_cmd(*color, false),
                    y_from_top(y + h)
                ));
            }
            DrawOp::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                color,
            } => {
                out.push_str(&format!(
                    "{} {:.2} w {x1:.2} {:.2} m {x2:.2} {:.2} l S\n",
                    color_cmd(*color, true),
                    width,
                    y_from_top(*y1),
                    y_from_top(*y2)
                ));
            }
            DrawOp::Image { x, y, w, h, id } => {
                out.push_str(&format!(
                    "q {w:.2} 0 0 {h:.2} {x:.2} {:.2} cm /Im{id} Do Q\n",
                    y_from_top(y + h)
                ));
            }
            DrawOp::Text(t) => {
                // inline code 优先尝试真实等宽字体；其缺字仍按主字体、Unicode 回退链处理。
                let mut fonts: Vec<&FontResource> = Vec::new();
                let mut names: Vec<String> = Vec::new();
                if t.monospace {
                    if let Some(mono) = monospace {
                        fonts.push(mono);
                        names.push("FM".into());
                    }
                }
                fonts.push(font);
                names.push("F1".into());
                for (i, f) in fallbacks.iter().enumerate() {
                    fonts.push(*f);
                    names.push(format!("F{}", i + 2));
                }
                let runs = split_font_runs_multi(&t.text, &fonts);
                let tw: f32 = runs
                    .iter()
                    .map(|(fi, s)| fonts[*fi].text_width(s, t.size))
                    .sum();
                if let Some(bg) = t.highlight {
                    out.push_str(&format!(
                        "{} {:.2} {:.2} {:.2} {:.2} re f\n",
                        color_cmd(bg, false),
                        t.x,
                        y_from_top(t.y + t.size * 0.28),
                        tw,
                        t.size * 1.28
                    ));
                }
                let tm_y = y_from_top(t.y);
                out.push_str("BT\n");
                if t.bold {
                    // 用「填充+描边」加粗，避免二次绘制造成笔画重影。
                    out.push_str(&format!("{}\n", color_cmd(t.color, true)));
                    out.push_str(&format!("{:.2} w 2 Tr\n", t.size * 0.028));
                }
                let mut x = t.x;
                for (fi, text) in runs {
                    let f = fonts[fi];
                    let tm = if t.italic {
                        format!("1 0 0.24 1 {x:.2} {tm_y:.2} Tm")
                    } else {
                        format!("1 0 0 1 {x:.2} {tm_y:.2} Tm")
                    };
                    let name = &names[fi];
                    out.push_str(&format!("/{name} {:.2} Tf\n", t.size));
                    out.push_str(&format!("{}\n", color_cmd(t.color, false)));
                    out.push_str(&format!("{tm}\n"));
                    out.push_str(&format!("{} Tj\n", f.encode_cid_hex(&text)));
                    x += f.text_width(&text, t.size);
                }
                if t.bold {
                    out.push_str("0 Tr\n");
                }
                out.push_str("ET\n");

                if t.underline {
                    out.push_str(&format!(
                        "{} {:.2} w {:.2} {:.2} m {:.2} {:.2} l S\n",
                        color_cmd(t.color, true),
                        0.6,
                        t.x,
                        y_from_top(t.y + t.size * 0.14),
                        t.x + tw,
                        y_from_top(t.y + t.size * 0.14)
                    ));
                }
                if t.strike {
                    let y = y_from_top(t.y + t.size * 0.32);
                    out.push_str(&format!(
                        "{} {:.2} w {:.2} {y:.2} m {:.2} {y:.2} l S\n",
                        color_cmd(t.color, true),
                        0.6,
                        t.x,
                        t.x + tw
                    ));
                }
            }
        }
    }
    out.into_bytes()
}

/// 写入一套 Type0/CIDFontType2 字体对象（主字体与回退字体共用）。
fn write_type0_font(
    pdf: &mut Pdf,
    font: &FontResource,
    font_desc_ref: Ref,
    cid_font_ref: Ref,
    type0_ref: Ref,
    tu_cmap_ref: Ref,
    font_stream_ref: Ref,
) {
    let base_name_bytes = font.base_name.as_bytes();
    pdf.font_descriptor(font_desc_ref)
        .name(Name(base_name_bytes))
        .flags(FontFlags::NON_SYMBOLIC)
        .bbox(Rect::new(
            font.font_bbox[0],
            font.font_bbox[1],
            font.font_bbox[2],
            font.font_bbox[3],
        ))
        .italic_angle(0.0)
        .ascent(font.ascent)
        .descent(font.descent)
        .stem_v(80.0)
        .font_file2(font_stream_ref);

    let widths = font.widths_by_cid();
    let mut cid = pdf.cid_font(cid_font_ref);
    cid.subtype(CidFontType::Type2)
        .base_font(Name(base_name_bytes))
        .system_info(pdf_writer::types::SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        })
        .font_descriptor(font_desc_ref)
        .default_width(1000.0)
        .cid_to_gid_map_predefined(Name(b"Identity"));
    // 不写 /W 时所有字形按 /DW=1000 步进，ASCII 会整体越出版心。
    cid.widths().consecutive(0, widths.iter().copied());
    drop(cid);

    pdf.stream(font_stream_ref, &font.subset_bytes);

    let cmap = build_to_unicode_cmap(&font.cid_to_char);
    pdf.stream(tu_cmap_ref, &cmap);

    let mut d = pdf.indirect(type0_ref).dict();
    d.pair(Name(b"Type"), Name(b"Font"));
    d.pair(Name(b"Subtype"), Name(b"Type0"));
    d.pair(Name(b"BaseFont"), Name(base_name_bytes));
    d.pair(Name(b"Encoding"), Name(b"Identity-H"));
    d.insert(Name(b"DescendantFonts"))
        .array()
        .push()
        .primitive(cid_font_ref);
    d.pair(Name(b"ToUnicode"), tu_cmap_ref);
    drop(d);
}

/// 为主字体缺字选择回退字体：按候选顺序逐个探测字形覆盖，把字符分配给
/// 第一个真正覆盖它的字体，并只为该字体子集化这些字符。
/// 返回顺序即 PDF 资源名顺序（/F2、/F3…）。
fn load_fallback_fonts(missing: &[char]) -> Vec<FontResource> {
    let mut remaining: BTreeSet<char> = missing.iter().copied().filter(|c| *c != '\0').collect();
    let mut fonts: Vec<FontResource> = Vec::new();
    for path in fallback_font_candidates() {
        if remaining.is_empty() {
            break;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
            continue;
        };
        let covered: Vec<char> = remaining
            .iter()
            .copied()
            .filter(|c| face.glyph_index(*c).is_some())
            .collect();
        if covered.is_empty() {
            continue;
        }
        let Ok(f) = FontResource::load_and_subset(Path::new(path), covered.iter().copied(), false)
        else {
            continue;
        };
        // 子集化后仍以实际可写出的 CID 为准（防御 ttf-parser 与 subsetter 的口径差异）。
        let actually: Vec<char> = covered.iter().copied().filter(|c| f.covers(*c)).collect();
        if actually.is_empty() {
            continue;
        }
        for c in &actually {
            remaining.remove(c);
        }
        fonts.push(f);
    }
    fonts
}

/// 把版面渲染为完整 PDF 字节。
pub fn render_pages(
    pages: &[Vec<DrawOp>],
    images: &[ImageData],
    outline: &[OutlineItem],
    links: &[LinkAnnot],
    font_path: &Path,
) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err(t(Key::PrintErrNoExportPages).into());
    }

    let mut chars: Vec<char> = Vec::new();
    for p in pages {
        for op in p {
            if let DrawOp::Text(t) = op {
                chars.extend(t.text.chars());
            }
        }
    }
    for o in outline {
        chars.extend(o.title.chars());
    }
    let font = FontResource::load_and_subset(font_path, chars.iter().copied(), false)?;

    // DEF-T09-E1-003：主字体缺字（emoji / CJK 扩展 B 等）时按「真实字形覆盖」
    // 逐个字符选择回退字体并子集化。不能只试 emoji 字体——`𠮷`(U+20BB7) 只
    // 由 SimSun-ExtB 等扩展区字体覆盖；也不能按文件名假设覆盖范围。
    let missing: Vec<char> = chars
        .iter()
        .copied()
        .filter(|c| *c != '\0' && !font.covers(*c))
        .collect();
    let fallback_fonts = load_fallback_fonts(&missing);
    let monospace_chars: BTreeSet<char> = pages
        .iter()
        .flatten()
        .filter_map(|op| match op {
            DrawOp::Text(t) if t.monospace => Some(t.text.chars()),
            _ => None,
        })
        .flatten()
        .collect();
    let monospace_font = if monospace_chars.is_empty() {
        None
    } else {
        find_monospace_font().and_then(|path| {
            FontResource::load_and_subset(Path::new(path), monospace_chars.iter().copied(), false)
                .ok()
        })
    };

    let mut pdf = Pdf::new();

    let page_tree_id = Ref::new(2);
    let font_desc_ref = Ref::new(5);
    let cid_font_ref = Ref::new(6);
    let type0_ref = Ref::new(7);
    let tu_cmap_ref = Ref::new(8);
    let font_stream_ref = Ref::new(9);

    // 页面引用与内容流引用依次分配
    let mut next_ref: i32 = 10;
    let mut page_refs: Vec<Ref> = Vec::new();
    let mut content_refs: Vec<Ref> = Vec::new();
    for _ in pages {
        page_refs.push(Ref::new(next_ref));
        next_ref += 1;
        content_refs.push(Ref::new(next_ref));
        next_ref += 1;
    }

    // 图片 XObject 引用
    let image_refs: Vec<Ref> = images
        .iter()
        .map(|_| {
            let r = Ref::new(next_ref);
            next_ref += 1;
            r
        })
        .collect();

    // 回退字体对象引用：每套字体 5 个（FontDescriptor/CIDFont/Type0/ToUnicode/字体流）
    let fallback_refs: Vec<(Ref, Ref, Ref, Ref, Ref)> = (0..fallback_fonts.len())
        .map(|_| {
            let mut next = || {
                let r = Ref::new(next_ref);
                next_ref += 1;
                r
            };
            (next(), next(), next(), next(), next())
        })
        .collect();

    let monospace_refs = monospace_font.as_ref().map(|_| {
        let mut next = || {
            let r = Ref::new(next_ref);
            next_ref += 1;
            r
        };
        (next(), next(), next(), next(), next())
    });

    let fb_refs: Vec<&FontResource> = fallback_fonts.iter().collect();

    // 链接注释：按页归组并按序分配引用（与 `page_links` 一一对应）。
    let mut page_links: Vec<Vec<&LinkAnnot>> = vec![Vec::new(); pages.len()];
    for l in links {
        if let Some(v) = page_links.get_mut(l.page) {
            v.push(l);
        }
    }
    let annot_refs: Vec<Vec<Ref>> = page_links
        .iter()
        .map(|v| {
            v.iter()
                .map(|_| {
                    let r = Ref::new(next_ref);
                    next_ref += 1;
                    r
                })
                .collect()
        })
        .collect();

    for (i, page) in pages.iter().enumerate() {
        let mut used: BTreeSet<usize> = BTreeSet::new();
        for op in page {
            if let DrawOp::Image { id, .. } = op {
                used.insert(*id);
            }
        }

        let mut page_obj = pdf.page(page_refs[i]);
        page_obj
            .parent(page_tree_id)
            .media_box(Rect::new(0.0, 0.0, A4_WIDTH, A4_HEIGHT))
            .contents(content_refs[i]);

        let mut res = page_obj.insert(Name(b"Resources")).dict();
        {
            let mut fonts = res.insert(Name(b"Font")).dict();
            fonts.pair(Name(b"F1"), type0_ref);
            for (k, r) in fallback_refs.iter().enumerate() {
                let name = format!("F{}", k + 2);
                fonts.pair(Name(name.as_bytes()), r.2);
            }
            if let Some(r) = monospace_refs.as_ref() {
                fonts.pair(Name(b"FM"), r.2);
            }
        }
        if !used.is_empty() && !image_refs.is_empty() {
            let mut xobj = res.insert(Name(b"XObject")).dict();
            for id in used {
                if id < image_refs.len() {
                    let name = format!("Im{id}");
                    xobj.pair(Name(name.as_bytes()), image_refs[id]);
                }
            }
        }
        drop(res);
        if !annot_refs[i].is_empty() {
            page_obj.annotations(annot_refs[i].iter().copied());
        }
        drop(page_obj);

        let content =
            content_stream_with_fonts_and_mono(page, &font, &fb_refs, monospace_font.as_ref());
        pdf.stream(content_refs[i], content.as_ref());
    }

    // —— 链接注释对象 ——
    // 外部链接用 URI 动作；文档内锚点/脚注用 GoTo + XYZ 目标。
    for (i, annots) in page_links.iter().enumerate() {
        for (r, l) in annot_refs[i].iter().zip(annots.iter()) {
            let mut a = pdf.annotation(*r);
            a.subtype(AnnotationType::Link)
                .rect(Rect::new(
                    l.x,
                    y_from_top(l.y + l.h),
                    l.x + l.w,
                    y_from_top(l.y),
                ))
                .border(0.0, 0.0, 0.0, None)
                .flags(AnnotationFlags::PRINT);
            match &l.target {
                LinkTarget::Uri(u) => {
                    let uri = uri_ascii(u);
                    a.action()
                        .action_type(ActionType::Uri)
                        .uri(Str(uri.as_bytes()));
                }
                LinkTarget::Internal { page, x, y } => {
                    let target = page_refs.get(*page).copied().unwrap_or(page_refs[0]);
                    a.action()
                        .action_type(ActionType::GoTo)
                        .destination()
                        .page(target)
                        .xyz(*x, y_from_top(*y), None);
                }
                // 未解析锚点不应出现在版面产物中；防御性跳过。
                LinkTarget::Anchor(_) => {}
            }
        }
    }

    pdf.pages(page_tree_id)
        .kids(page_refs.iter().copied())
        .count(page_refs.len() as i32);

    // —— 图片对象 ——
    for (i, img) in images.iter().enumerate() {
        write_image_xobject(&mut pdf, image_refs[i], img);
    }

    // —— 字体 ——
    write_type0_font(
        &mut pdf,
        &font,
        font_desc_ref,
        cid_font_ref,
        type0_ref,
        tu_cmap_ref,
        font_stream_ref,
    );
    for (fb, (fd, cid_r, type0_r, tu, stream)) in fallback_fonts.iter().zip(fallback_refs) {
        write_type0_font(&mut pdf, fb, fd, cid_r, type0_r, tu, stream);
    }
    if let (Some(mono), Some((fd, cid_r, type0_r, tu, stream))) =
        (monospace_font.as_ref(), monospace_refs)
    {
        write_type0_font(&mut pdf, mono, fd, cid_r, type0_r, tu, stream);
    }

    // —— 书签（多级）——
    let outline_root_ref = if !outline.is_empty() {
        let root_ref = Ref::new(next_ref);
        next_ref += 1;
        let item_refs: Vec<Ref> = outline
            .iter()
            .map(|_| {
                let r = Ref::new(next_ref);
                next_ref += 1;
                r
            })
            .collect();

        let mut children: Vec<Vec<usize>> = vec![Vec::new(); outline.len()];
        let mut roots: Vec<usize> = Vec::new();
        let mut parent_of: Vec<Option<usize>> = vec![None; outline.len()];
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..outline.len() {
            while let Some(&top) = stack.last() {
                if outline[top].level >= outline[i].level {
                    stack.pop();
                } else {
                    break;
                }
            }
            parent_of[i] = stack.last().copied();
            match stack.last() {
                Some(&p) => children[p].push(i),
                None => roots.push(i),
            }
            stack.push(i);
        }

        // 兄弟链：prev/next 只在同一父级内连边
        let mut prev_of: Vec<Option<usize>> = vec![None; outline.len()];
        let mut next_of: Vec<Option<usize>> = vec![None; outline.len()];
        for group in children.iter().chain(std::iter::once(&roots)) {
            for w in group.windows(2) {
                next_of[w[0]] = Some(w[1]);
                prev_of[w[1]] = Some(w[0]);
            }
        }

        for (i, entry) in outline.iter().enumerate() {
            let page_idx = entry.page_idx.min(page_refs.len() - 1);
            let page_ref = page_refs[page_idx];
            let pdf_y = y_from_top(entry.top_y);

            let parent_ref = parent_of[i].map(|p| item_refs[p]).unwrap_or(root_ref);

            let mut item = pdf.outline_item(item_refs[i]);
            item.title(pdf_writer::TextStr(&entry.title));
            item.parent(parent_ref);
            if let Some(p) = prev_of[i] {
                item.prev(item_refs[p]);
            }
            if let Some(n) = next_of[i] {
                item.next(item_refs[n]);
            }
            if !children[i].is_empty() {
                let first = item_refs[children[i][0]];
                let last = item_refs[*children[i].last().unwrap()];
                item.first(first);
                item.last(last);
                item.count(children[i].len() as i32);
            }
            item.dest().page(page_ref).xyz(0.0, pdf_y, Some(1.0));
            drop(item);
        }

        let mut root = pdf.outline(root_ref);
        if let Some(&first) = roots.first() {
            root.first(item_refs[first]);
        }
        if let Some(&last) = roots.last() {
            root.last(item_refs[last]);
        }
        root.count(roots.len() as i32);
        drop(root);

        Some(root_ref)
    } else {
        None
    };

    let catalog_id = Ref::new(1);
    let mut catalog = pdf.catalog(catalog_id);
    catalog.pages(page_tree_id);
    if let Some(ref r) = outline_root_ref {
        catalog.outlines(*r);
    }
    drop(catalog);

    Ok(pdf.finish())
}

// =============================================================================
// 单元测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_dimensions_are_correct() {
        assert!((A4_WIDTH - 595.28).abs() < 0.01);
        assert!((A4_HEIGHT - 841.89).abs() < 0.01);
    }

    #[test]
    fn y_from_top_inverts_correctly() {
        assert!((y_from_top(0.0) - A4_HEIGHT).abs() < 0.01);
        assert!((y_from_top(A4_HEIGHT) - 0.0).abs() < 0.01);
    }

    #[test]
    fn to_unicode_cmap_formats_correctly() {
        let cid_to_char = vec!['\0', 'A', '中', '\0', '文', '😀'];
        let cmap = build_to_unicode_cmap(&cid_to_char);
        let s = String::from_utf8_lossy(&cmap);
        assert!(s.contains("/CIDSystemInfo"));
        assert!(s.contains("beginbfchar"));
        assert!(s.contains("<0001> <0041>"));
        assert!(s.contains("<0002> <4E2D>"));
        assert!(s.contains("<0004> <6587>"));
        assert!(
            s.contains("<0005> <D83DDE00>"),
            "BMP 外字符应映射为 UTF-16 代理对"
        );
    }

    #[test]
    fn to_unicode_cmap_chunks_by_100() {
        // 120 个 BMP 字符应拆成 100 + 20 两个块，单块不得超过规范上限。
        let chars: Vec<char> = (0x4E00u32..0x4E00 + 120)
            .filter_map(char::from_u32)
            .collect();
        let mut cid_to_char = vec!['\0'];
        cid_to_char.extend(chars);
        let s = String::from_utf8_lossy(&build_to_unicode_cmap(&cid_to_char)).to_string();
        assert!(s.contains("100 beginbfchar"));
        assert!(s.contains("20 beginbfchar"));
    }

    #[test]
    fn cid_fallback_and_widths_by_cid() {
        let mut char_to_cid = HashMap::new();
        char_to_cid.insert('\0', 0u16);
        char_to_cid.insert('?', 1u16);
        let mut widths = HashMap::new();
        widths.insert('?', 500.0f32);
        let font = FontResource {
            char_to_cid,
            cid_to_char: vec!['\0', '?'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "t".into(),
            widths,
        };
        // 无字形字符不得落到 CID 0/.notdef（否则宽度按 1em 且无 ToUnicode）。
        assert_eq!(font.cid_of('😀'), 1);
        assert_eq!(font.encode_cid_hex("A"), "<0001>");
        // /W 数组按 CID 顺序：CID 0 用 1000，CID 1 用真实 500。
        assert_eq!(font.widths_by_cid(), vec![1000.0, 500.0]);
    }

    #[test]
    fn content_stream_emits_ops() {
        let font = FontResource {
            char_to_cid: HashMap::new(),
            cid_to_char: vec!['\0'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "t".into(),
            widths: HashMap::new(),
        };
        let page = vec![
            DrawOp::Rect {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 40.0,
                color: [255, 0, 0],
            },
            DrawOp::Text(TextRun {
                x: 50.0,
                y: 100.0,
                size: 11.0,
                text: "A".into(),
                bold: true,
                italic: false,
                monospace: false,
                underline: true,
                strike: false,
                highlight: Some([240, 240, 240]),
                color: [0, 0, 0],
            }),
        ];
        let s = String::from_utf8_lossy(&content_stream(&page, &font, None)).to_string();
        assert!(s.contains("re f"), "矩形应输出 re f");
        assert!(s.contains("BT") && s.contains("Tj"), "文本应输出 BT/Tj");
        assert!(s.contains("Tm"), "文本应使用 Tm 定位");
    }

    #[cfg(windows)]
    #[test]
    fn font_candidates_exist_on_windows() {
        assert!(find_cjk_font().is_some(), "Windows 上应有可用 CJK 字体");
    }

    /// DEF-T09-E1-003：主字体缺字时按 run 切换到回退字体（/F2），
    /// 内容流保留真实 CID（而非 `?`），复制文本不失真。
    #[test]
    fn content_stream_switches_to_fallback_font_for_missing_glyphs() {
        let primary = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('A', 1), ('?', 2)]),
            cid_to_char: vec!['\0', 'A', '?'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "primary".into(),
            widths: HashMap::from([('A', 600.0f32), ('?', 500.0)]),
        };
        let fallback = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('😀', 1)]),
            cid_to_char: vec!['\0', '😀'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "emoji".into(),
            widths: HashMap::from([('😀', 1000.0f32)]),
        };
        let page = vec![DrawOp::Text(TextRun {
            x: 50.0,
            y: 100.0,
            size: 11.0,
            text: "A😀A".into(),
            bold: false,
            italic: false,
            monospace: false,
            underline: false,
            strike: false,
            highlight: None,
            color: [0, 0, 0],
        })];
        let s =
            String::from_utf8_lossy(&content_stream(&page, &primary, Some(&fallback))).to_string();
        assert!(s.contains("/F1"), "应有主字体 run: {s}");
        let after_fallback = s.split("/F2").nth(1).expect("emoji 应切到回退字体");
        assert!(
            after_fallback.contains("<0001> Tj"),
            "emoji 应写回退字体真实 CID 而非 ?: {s}"
        );
        assert!(
            !s.contains("<0002> Tj"),
            "不得把 emoji 降级为 `?` 的 CID: {s}"
        );
        // 同一文本在主字体下只写 /F1（回归：不要无条件引用 /F2）。
        let s = String::from_utf8_lossy(&content_stream(&page[..1], &primary, None)).to_string();
        assert!(!s.contains("/F2"), "{s}");
    }

    #[test]
    fn monospace_content_uses_fm_real_widths_and_unicode_fallbacks() {
        let primary = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('i', 1), ('m', 2), ('中', 3), ('?', 4)]),
            cid_to_char: vec!['\0', 'i', 'm', '中', '?'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "primary".into(),
            widths: HashMap::from([('i', 300.0f32), ('m', 700.0), ('中', 1000.0), ('?', 500.0)]),
        };
        let mono = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('i', 1), ('m', 2)]),
            cid_to_char: vec!['\0', 'i', 'm'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "mono".into(),
            widths: HashMap::from([('i', 600.0f32), ('m', 600.0)]),
        };
        let fallback = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('😀', 1)]),
            cid_to_char: vec!['\0', '😀'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "emoji".into(),
            widths: HashMap::from([('😀', 1000.0f32)]),
        };
        let run = TextRun {
            x: 50.0,
            y: 100.0,
            size: 10.0,
            text: "im中😀".into(),
            bold: false,
            italic: false,
            monospace: true,
            underline: false,
            strike: false,
            highlight: Some([240, 240, 240]),
            color: [0, 0, 0],
        };
        let fallbacks = [&fallback];
        let page = vec![DrawOp::Text(run.clone())];
        let s = String::from_utf8_lossy(&content_stream_with_fonts_and_mono(
            &page,
            &primary,
            &fallbacks,
            Some(&mono),
        ))
        .to_string();
        let fm = s.find("/FM 10.00 Tf").expect("ASCII 应用等宽字体");
        let f1 = s.find("/F1 10.00 Tf").expect("中文应回退主字体");
        let f2 = s.find("/F2 10.00 Tf").expect("emoji 应回退 Unicode 字体");
        assert!(fm < f1 && f1 < f2, "字体链顺序错误：{s}");
        assert!(s.contains("1 0 0 1 62.00 741.89 Tm"), "等宽步进错误：{s}");
        assert!(s.contains("1 0 0 1 72.00 741.89 Tm"), "回退步进错误：{s}");
        assert!(
            s.contains("32.00 12.80 re f"),
            "浅底宽度须匹配真实字宽：{s}"
        );
        assert!(
            s.contains("<0003> Tj") && s.contains("<0001> Tj"),
            "缺字不得变成 ?：{s}"
        );

        let normal_page = vec![DrawOp::Text(TextRun {
            monospace: false,
            ..run
        })];
        let normal = String::from_utf8_lossy(&content_stream_with_fonts_and_mono(
            &normal_page,
            &primary,
            &fallbacks,
            Some(&mono),
        ))
        .to_string();
        assert!(
            !normal.contains("/FM"),
            "普通正文不得使用等宽资源：{normal}"
        );
        assert!(
            normal.contains("30.00 12.80 re f"),
            "普通正文仍用比例字宽：{normal}"
        );
    }

    #[test]
    fn split_font_runs_groups_by_coverage() {
        let primary = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('A', 1)]),
            cid_to_char: vec!['\0', 'A'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "p".into(),
            widths: HashMap::new(),
        };
        let fallback = FontResource {
            char_to_cid: HashMap::from([('\0', 0u16), ('😀', 1)]),
            cid_to_char: vec!['\0', '😀'],
            subset_bytes: Vec::new(),
            font_bbox: [0.0; 4],
            ascent: 800.0,
            descent: -200.0,
            base_name: "f".into(),
            widths: HashMap::new(),
        };
        let runs = split_font_runs("AA😀😀A", &primary, Some(&fallback));
        assert_eq!(
            runs,
            vec![
                (false, "AA".to_string()),
                (true, "😀😀".to_string()),
                (false, "A".to_string()),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn render_pages_embeds_emoji_fallback_font() {
        // DEF-T09-E1-003：真实系统字体路径下，emoji 走 /F2 且 ToUnicode 含代理对。
        let Some(fb_path) = find_fallback_font() else {
            return; // 无 emoji 字体的环境不强行失败
        };
        assert!(Path::new(fb_path).exists());
        let pages = vec![vec![DrawOp::Text(TextRun {
            x: 56.0,
            y: 80.0,
            size: 11.0,
            text: "表情 😀 结束".into(),
            bold: false,
            italic: false,
            monospace: false,
            underline: false,
            strike: false,
            highlight: None,
            color: [0, 0, 0],
        })]];
        let bytes = render_pages(&pages, &[], &[], &[], Path::new(find_cjk_font().unwrap()))
            .expect("渲染 PDF");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/F2"), "应嵌入并使用回退字体");
        assert!(s.contains("<D83DDE00>"), "ToUnicode 应映射 emoji 代理对");
    }

    #[cfg(windows)]
    #[test]
    fn inline_code_uses_real_monospace_metrics_and_embedded_font() {
        let Some(mono_path) = find_monospace_font() else {
            return; // 无等宽字体的环境明确回退，不用比例字体冒充
        };
        let font_path = find_cjk_font().unwrap();
        let metrics = FontMetrics::load(Path::new(font_path), ['i', 'm', '中']).expect("字体度量");
        let mono_bytes = std::fs::read(mono_path).expect("读取等宽字体");
        let mono_face = ttf_parser::Face::parse(&mono_bytes, 0).expect("解析等宽字体");
        let mono_scale = 1000.0 / mono_face.units_per_em().max(1) as f32;
        let advance = |c| {
            mono_face
                .glyph_index(c)
                .and_then(|gid| mono_face.glyph_hor_advance(gid))
                .expect("等宽字体应覆盖 ASCII") as f32
                * mono_scale
        };
        let size = 10.0;
        let i = metrics.width_of_monospace('i', size).expect("等宽 i 度量");
        let m = metrics.width_of_monospace('m', size).expect("等宽 m 度量");
        assert!((i - advance('i') * size / 1000.0).abs() < 0.001);
        assert!((m - advance('m') * size / 1000.0).abs() < 0.001);
        assert!((i - m).abs() < 0.001, "真实等宽字体的 i/m advance 必须一致");
        if mono_face.glyph_index('中').is_none() {
            assert!(metrics.width_of_monospace('中', size).is_none());
            assert!(
                (metrics.width_of_style('中', size, true) - metrics.width_of('中', size)).abs()
                    < 0.001,
                "等宽字体缺字必须复用主字体/Unicode 回退宽度"
            );
        }

        let pages = vec![vec![DrawOp::Text(TextRun {
            x: 56.0,
            y: 80.0,
            size: 11.0,
            text: "im中".into(),
            bold: false,
            italic: false,
            monospace: true,
            underline: false,
            strike: false,
            highlight: Some([243, 244, 246]),
            color: [0, 0, 0],
        })]];
        let bytes = render_pages(&pages, &[], &[], &[], Path::new(font_path)).expect("渲染 PDF");
        let s = String::from_utf8_lossy(&bytes);
        assert!(
            s.matches("/FM").count() >= 2,
            "页面资源与内容流均应使用 /FM"
        );
        let base_name = Path::new(mono_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("等宽字体名");
        assert!(
            s.contains(&format!("/BaseFont /{base_name}")),
            "应嵌入真实等宽字体 {base_name}"
        );
    }

    /// DEF-T09-E1-003 残留：`𠮷`（U+20BB7，CJK 扩展 B）主字体缺字，
    /// 必须由真正覆盖该码点的系统字体承载：内容流用回退字体真实 CID，
    /// ToUnicode 输出原码点（而不是 `?` / 替换字符）。
    #[cfg(windows)]
    #[test]
    fn cjk_ext_b_falls_back_to_covering_font_with_correct_to_unicode() {
        let font_path = Path::new(find_cjk_font().unwrap());
        let primary = FontResource::load_and_subset(font_path, ['𠮷'], false).expect("主字体子集");
        assert!(
            !primary.covers('𠮷'),
            "回归前提：主字体（雅黑/等线链）不覆盖 U+20BB7"
        );
        let fallbacks = load_fallback_fonts(&['𠮷']);
        let fb = fallbacks
            .iter()
            .find(|f| f.covers('𠮷'))
            .expect("系统回退字体必须真实覆盖 U+20BB7（如 SimSun-ExtB）");
        assert!(
            !fb.base_name.contains("seguiemj"),
            "覆盖字体不能是 emoji 字体（其无该字形），实际：{}",
            fb.base_name
        );

        let page = vec![DrawOp::Text(TextRun {
            x: 56.0,
            y: 80.0,
            size: 11.0,
            text: "𠮷".into(),
            bold: false,
            italic: false,
            monospace: false,
            underline: false,
            strike: false,
            highlight: None,
            color: [0, 0, 0],
        })];
        let refs: Vec<&FontResource> = fallbacks.iter().collect();
        let content = String::from_utf8_lossy(&content_stream_with_fonts_and_mono(
            &page, &primary, &refs, None,
        ))
        .to_string();
        let cid = fb.char_to_cid[&'𠮷'];
        assert!(
            content.contains(&format!("<{cid:04X}> Tj")),
            "内容流应写回退字体的真实 CID：{content}"
        );
        assert!(
            !content.contains("<0001> Tj"),
            "不得把 U+20BB7 降级为 `?` 的 CID：{content}"
        );
        assert!(
            String::from_utf8_lossy(&build_to_unicode_cmap(&fb.cid_to_char)).contains("<D842DFB7>"),
            "ToUnicode 应映射 𠮷 的 UTF-16 代理对 D842 DFB7"
        );

        // 端到端：完整 PDF 的 ToUnicode 流可复制出原字符。
        let bytes = render_pages(&[page], &[], &[], &[], font_path).expect("渲染 PDF");
        assert!(String::from_utf8_lossy(&bytes).contains("<D842DFB7>"));
    }

    #[cfg(windows)]
    #[test]
    fn render_pages_produces_valid_pdf_with_outline() {
        let font_path = find_cjk_font().unwrap();
        let pages = vec![
            vec![DrawOp::Text(TextRun {
                x: 56.0,
                y: 80.0,
                size: 22.0,
                text: "第一章 引言".into(),
                bold: true,
                italic: false,
                monospace: false,
                underline: false,
                strike: false,
                highlight: None,
                color: [0, 0, 0],
            })],
            vec![DrawOp::Text(TextRun {
                x: 56.0,
                y: 80.0,
                size: 18.0,
                text: "第二章 实现".into(),
                bold: true,
                italic: false,
                monospace: false,
                underline: false,
                strike: false,
                highlight: None,
                color: [0, 0, 0],
            })],
        ];
        let outline = vec![
            OutlineItem {
                title: "第一章 引言".into(),
                level: 1,
                page_idx: 0,
                top_y: 64.0,
            },
            OutlineItem {
                title: "第二章 实现".into(),
                level: 2,
                page_idx: 1,
                top_y: 64.0,
            },
        ];
        let bytes =
            render_pages(&pages, &[], &outline, &[], Path::new(font_path)).expect("应产出 PDF");
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.ends_with(b"%%EOF"));
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/Type0"));
        assert!(s.contains("/CIDFontType2"));
        assert!(s.contains("/ToUnicode"));
        assert!(s.contains("/Outlines"));
        assert!(s.contains("/First"), "书签应有 first（层级树）");
        assert!(s.contains("/Dest"));
    }

    /// B7：H1/H2/H3 中文书签的 PDF 目录对象必须形成正确层级树，
    /// 标题文本按 UTF-16BE 写入（不丢中文），页数/书签数与输入一致。
    #[cfg(windows)]
    #[test]
    fn pdf_outline_nests_h1_h2_h3_and_keeps_chinese_titles() {
        let font_path = find_cjk_font().unwrap();
        let page = |t: &str| {
            vec![DrawOp::Text(TextRun {
                x: 56.0,
                y: 80.0,
                size: 18.0,
                text: t.into(),
                bold: true,
                italic: false,
                monospace: false,
                underline: false,
                strike: false,
                highlight: None,
                color: [0, 0, 0],
            })]
        };
        let pages = vec![page("第一章 引言"), page("1.1 背景")];
        let outline = vec![
            OutlineItem {
                title: "第一章 引言".into(),
                level: 1,
                page_idx: 0,
                top_y: 64.0,
            },
            OutlineItem {
                title: "1.1 背景".into(),
                level: 2,
                page_idx: 1,
                top_y: 64.0,
            },
            OutlineItem {
                title: "1.1.1 细节".into(),
                level: 3,
                page_idx: 1,
                top_y: 200.0,
            },
            OutlineItem {
                title: "1.2 目标".into(),
                level: 2,
                page_idx: 1,
                top_y: 320.0,
            },
        ];
        let bytes =
            render_pages(&pages, &[], &outline, &[], Path::new(font_path)).expect("应产出 PDF");
        let s = String::from_utf8_lossy(&bytes).to_uppercase();
        assert_eq!(s.matches("/TITLE").count(), outline.len(), "书签数=标题数");
        assert_eq!(
            s.matches("/MEDIABOX").count(),
            pages.len(),
            "页数=页面对象数"
        );
        // UTF-16BE（pdf_writer 带 BOM）：中文标题的码元必须原样出现，书签不乱码。
        for title in ["第一章 引言", "1.1 背景", "1.1.1 细节", "1.2 目标"] {
            let hex: String = title
                .encode_utf16()
                .map(|u| format!("{u:04X}"))
                .collect::<Vec<_>>()
                .join("");
            assert!(
                s.contains(&hex),
                "书签标题 {title:?} 必须以 UTF-16BE 写入，缺失 {hex}"
            );
        }
        // 层级树：H1 下有 2 个 H2；第一个 H2 下有 1 个 H3。
        assert!(s.contains("/COUNT 2"), "H1 应有 2 个直接子书签");
        assert!(s.contains("/COUNT 1"), "H2/H3 应有 1 个直接子书签");
        assert!(
            s.matches("/PARENT").count() >= outline.len(),
            "每个书签须有父节点"
        );
        assert!(s.contains("/DEST"), "书签须带跳转目标");
    }

    /// Task 5：外链写 URI 动作、内链写 GoTo + XYZ 目标，页对象挂 /Annots。
    #[cfg(windows)]
    #[test]
    fn pdf_writes_link_annotations_for_uri_and_internal_targets() {
        let font_path = find_cjk_font().unwrap();
        let page = vec![DrawOp::Text(TextRun {
            x: 56.0,
            y: 80.0,
            size: 11.0,
            text: "链接".into(),
            bold: false,
            italic: false,
            monospace: false,
            underline: true,
            strike: false,
            highlight: None,
            color: [37, 99, 235],
        })];
        let links = vec![
            LinkAnnot {
                page: 0,
                x: 40.0,
                y: 80.0,
                w: 60.0,
                h: 12.0,
                target: LinkTarget::Uri("https://example.com/guide".into()),
            },
            LinkAnnot {
                page: 0,
                x: 40.0,
                y: 100.0,
                w: 60.0,
                h: 12.0,
                target: LinkTarget::Internal {
                    page: 0,
                    x: 40.0,
                    y: 120.0,
                },
            },
        ];
        let bytes =
            render_pages(&[page], &[], &[], &links, Path::new(font_path)).expect("应产出 PDF");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/Annots"), "页面须挂注释数组");
        assert_eq!(s.matches("/Subtype /Link").count(), 2, "两条链接注释");
        assert!(s.contains("/URI (https://example.com/guide)"));
        assert!(s.contains("/GoTo"), "内链使用 GoTo 动作");
        assert!(s.contains("/XYZ"), "内链目标须带 XYZ 坐标");
        assert_eq!(s.matches("/Rect").count(), 2, "每条注释须有矩形");
    }

    // =========================================================================
    // 图片 Flate 压缩（Task 6.1）
    // =========================================================================

    /// 构造一张接近 Mermaid 输出的近平坦 RGB 图：浅色渐变底 + 少量深色块。
    fn mermaid_like_rgb(w: u32, h: u32) -> Vec<u8> {
        let (wu, hu) = (w as usize, h as usize);
        let mut rgb = vec![0u8; wu * hu * 3];
        for y in 0..hu {
            for x in 0..wu {
                let i = (y * wu + x) * 3;
                let shade = (238 + x * 12 / wu.max(1)).min(255) as u8;
                rgb[i] = shade;
                rgb[i + 1] = shade.saturating_add(2);
                rgb[i + 2] = 252;
            }
        }
        let blocks = [
            (100usize, 200usize, 600usize, 24usize, [32u8, 32, 32]),
            (120, 600, 400, 40, [64, 64, 64]),
            (300, 1200, 900, 16, [40, 90, 160]),
        ];
        for (bx, by, bw, bh, color) in blocks {
            for y in by..(by + bh).min(hu) {
                for x in bx..(bx + bw).min(wu) {
                    let i = (y * wu + x) * 3;
                    rgb[i..i + 3].copy_from_slice(&color);
                }
            }
        }
        rgb
    }

    /// 最坏情况：高噪声 RGB（deflate 几乎无法压缩）。
    fn noise_rgb(w: u32, h: u32) -> Vec<u8> {
        let mut state = 0x1234_5678u32;
        let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
        for _ in 0..(w as usize * h as usize * 3) {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            rgb.push((state >> 16) as u8);
        }
        rgb
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + data.len());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }

    /// 用生产代码产出的 zlib 流重新拼一个最小 PNG，供 `png::Decoder` 回读校验。
    fn assemble_png(w: u32, h: u32, zlib: &[u8]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB / deflate / no interlace
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
        png.extend_from_slice(&png_chunk(b"IDAT", zlib));
        png.extend_from_slice(&png_chunk(b"IEND", &[]));
        png
    }

    fn decode_png_rgb(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().expect("解析 PNG 头");
        let mut buf = vec![0; reader.output_buffer_size().expect("输出缓冲大小")];
        let info = reader.next_frame(&mut buf).expect("解码 PNG 帧");
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    #[test]
    fn large_flat_rgb_compresses_under_one_tenth_and_roundtrips() {
        let (w, h) = (1520u32, 2190u32);
        let rgb = mermaid_like_rgb(w, h);
        assert_eq!(rgb.len(), 1520 * 2190 * 3);
        let compressed = compress_image_samples(&rgb, w, h).expect("大图应产出 zlib 流");
        assert!(
            compressed.len() * 10 < rgb.len(),
            "近平坦图压缩后应小于原始 1/10：{} vs {}",
            compressed.len(),
            rgb.len()
        );
        let (dw, dh, decoded) = decode_png_rgb(&assemble_png(w, h, &compressed));
        assert_eq!((dw, dh), (w, h));
        assert_eq!(decoded, rgb, "PNG 解码必须逐字节还原原始 RGB");
    }

    #[test]
    fn noisy_rgb_compresses_losslessly_without_panic() {
        let (w, h) = (256u32, 256u32);
        let rgb = noise_rgb(w, h);
        assert!(rgb.len() >= IMAGE_COMPRESS_MIN_BYTES);
        let compressed = compress_image_samples(&rgb, w, h).expect("高噪声图仍应可编码");
        let (_, _, decoded) = decode_png_rgb(&assemble_png(w, h, &compressed));
        assert_eq!(decoded, rgb, "最坏情况的噪声图也必须无损回读");
    }

    #[test]
    fn small_image_is_left_uncompressed() {
        let rgb = vec![0u8; 32 * 32 * 3];
        assert!(rgb.len() < IMAGE_COMPRESS_MIN_BYTES);
        assert!(compress_image_samples(&rgb, 32, 32).is_none());
    }

    #[test]
    fn pdf_image_xobject_uses_flate_and_png_predictor() {
        let (w, h) = (1520u32, 2190u32);
        let img = ImageData {
            w,
            h,
            rgb: mermaid_like_rgb(w, h),
            baseline_px: 0.0,
            px_to_pt: 0.5,
        };
        let mut pdf = Pdf::new();
        write_image_xobject(&mut pdf, Ref::new(1), &img);
        let bytes = pdf.finish();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("/Filter /FlateDecode"), "{s}");
        assert!(s.contains("/Predictor 15"), "PNG predictor 应为 15: {s}");
        assert!(s.contains("/Colors 3"), "{s}");
        assert!(s.contains("/BitsPerComponent 8"), "{s}");
        assert!(s.contains("/Columns 1520"), "{s}");
        assert!(
            !s.contains("/Length 9986400"),
            "不得再写入未压缩的原始 RGB 流"
        );
    }

    #[cfg(windows)]
    #[test]
    fn render_pages_compresses_large_mermaid_like_image() {
        let (w, h) = (1520u32, 2190u32);
        let raw = mermaid_like_rgb(w, h);
        let img = ImageData {
            w,
            h,
            rgb: raw.clone(),
            baseline_px: 0.0,
            px_to_pt: 0.5,
        };
        let pages = vec![vec![DrawOp::Image {
            x: 56.0,
            y: 80.0,
            w: 400.0,
            h: 576.0,
            id: 0,
        }]];
        let bytes = render_pages(
            &pages,
            &[img],
            &[],
            &[],
            Path::new(find_cjk_font().unwrap()),
        )
        .expect("渲染 PDF");
        assert!(
            bytes.len() < 1024 * 1024,
            "含大图的 PDF 应显著小于原始位图：{} vs {}",
            bytes.len(),
            raw.len()
        );
        assert!(String::from_utf8_lossy(&bytes).contains("/Predictor 15"));
    }
}
