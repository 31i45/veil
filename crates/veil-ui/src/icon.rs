// 应用图标与文档图标（单一真源）：纯代码生成、零二进制资产。
// 同一份像素两处复用（本文件不能含 `//!` 内部文档注释：build.rs 以 include! 嵌入）：
// - veil-ui 窗口图标：app.rs 调 `icon_rgba` 喂 winit `with_window_icon`；
// - veil build.rs 生成 ICO 文件资源：`include!` 本文件后调 `icon_bytes` / `doc_icon_bytes`。
// 改色/改形只改本文件。
//
// 设计：应用图标 = 折角纸页 + V 形面纱揭页 + 文字行（面纱隐喻「光标展开标记揭示源码」）；
// 文档图标 = 折角纸页 + M↓ 徽标。两者共享同一套页面几何（page_sdf）。

// ---- 色板（f32, 0..1） ----
const PAGE: [f32; 3] = [242.0 / 255.0, 233.0 / 255.0, 216.0 / 255.0]; // 暖羊皮纸 #F2E9D8
const FOLD: [f32; 3] = [220.0 / 255.0, 208.0 / 255.0, 185.0 / 255.0]; // 折角背页 #DCD0B9
const VEIL_LIGHT: [f32; 3] = [79.0 / 255.0, 70.0 / 255.0, 229.0 / 255.0]; // 面纱亮 #4F46E5 (indigo-600)
const VEIL_DARK: [f32; 3] = [55.0 / 255.0, 48.0 / 255.0, 163.0 / 255.0]; // 面纱暗 #3730A3 (indigo-800)
const INK: [f32; 3] = [30.0 / 255.0, 27.0 / 255.0, 75.0 / 255.0]; // 墨色 #1E1B4B

/// RGBA 像素（自顶向下，每像素 4 字节），供 winit 窗口图标。
pub fn icon_rgba(s: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((s * s * 4) as usize);
    for y in 0..s {
        for x in 0..s {
            let [b, g, r, a] = icon_pixel(x as f32 + 0.5, y as f32 + 0.5, s as f32);
            px.extend_from_slice(&[r, g, b, a]);
        }
    }
    px
}

/// 应用图标 ICO（16/20/24/32/48/64/256 七档，BMP-DIB 帧，无 zlib 依赖）。
#[allow(dead_code)]
pub fn icon_bytes() -> Vec<u8> {
    build_ico(&[16, 20, 24, 32, 48, 64, 256], icon_pixel)
}

/// 文档图标 ICO（16/20/24/32/48/64/256 七档，BMP-DIB 帧）。
#[allow(dead_code)]
pub fn doc_icon_bytes() -> Vec<u8> {
    build_ico(&[16, 20, 24, 32, 48, 64, 256], doc_icon_pixel)
}

/// ICO 容器：目录头 + 多帧 BMP-DIB（32bpp，含 AND 掩码）。
fn build_ico(sizes: &[u32], pixel_fn: fn(f32, f32, f32) -> [u8; 4]) -> Vec<u8> {
    let mut frames = Vec::new();
    for s in sizes {
        frames.push(icon_frame(*s, pixel_fn));
    }
    let mut out = Vec::with_capacity(6 + 16 * sizes.len() + frames.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type=icon
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, f) in sizes.iter().zip(&frames) {
        out.push(if *s == 256 { 0 } else { *s as u8 }); // 256 写 0
        out.push(if *s == 256 { 0 } else { *s as u8 });
        out.extend_from_slice(&[0, 0]); // 调色板数、保留
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += f.len() as u32;
    }
    for f in frames {
        out.extend_from_slice(&f);
    }
    out
}

/// 单帧：BITMAPINFOHEADER（高 = 2×，含 AND 掩码）+ 自底向上 BGRA + 全零掩码。
fn icon_frame(s: u32, pixel_fn: fn(f32, f32, f32) -> [u8; 4]) -> Vec<u8> {
    let mut px = Vec::with_capacity((s * s * 4) as usize);
    for y in (0..s).rev() {
        // 自底向上
        for x in 0..s {
            px.extend_from_slice(&pixel_fn(x as f32 + 0.5, y as f32 + 0.5, s as f32));
        }
    }
    let mask_row = (s.div_ceil(32) * 4) as usize;
    let mut out = Vec::with_capacity(40 + px.len() + mask_row * s as usize);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(s as i32).to_le_bytes());
    out.extend_from_slice(&((s as i32) * 2).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&px.len().to_le_bytes());
    out.extend_from_slice(&[0; 16]); // 分辨率与调色板字段
    out.extend_from_slice(&px);
    out.resize(out.len() + mask_row * s as usize, 0);
    out
}

/// 共享页面几何：圆角矩形 + 右上折角三角形。
/// 返回 (body_sdf, fold_signed_dist, fold_inside)：
/// body_sdf < 0 在页面主体内；fold_signed_dist < 0 在折角三角内。
fn page_geometry(x: f32, y: f32, s: f32) -> (f32, f32, bool) {
    let margin = 0.06 * s;
    let (l, t, r, b) = (margin, margin, s - margin, s - margin);
    let radius = 0.10 * s;
    let d_rect = rounded_rect_sdf(x, y, l, t, r, b, radius);

    let fold = 0.18 * s;
    // 折线从 (r-fold, t) 到 (r, t+fold)；折角三角在该线朝向 (r,t) 一侧。
    let fold_cross = (x - (r - fold)) * fold - (y - t) * fold;
    let fold_inside = fold_cross >= 0.0;
    let d_fold_line = seg_dist(x, y, r - fold, t, r, t + fold);
    let signed_d_fold = if fold_inside { -d_fold_line } else { d_fold_line };

    // 页面主体 = 圆角矩形 减去 折角三角。
    let d_body = if fold_inside {
        d_rect.max(d_fold_line)
    } else {
        d_rect
    };
    (d_body, signed_d_fold, fold_inside)
}

/// 应用图标单像素 BGRA：折角纸页 + V 形面纱揭页 + 文字行。
fn icon_pixel(x: f32, y: f32, s: f32) -> [u8; 4] {
    let (d_body, signed_d_fold, _) = page_geometry(x, y, s);
    let page_a = clamp01(0.5 - d_body);

    // 基底：纸页色，折角处用背页色。
    let fold_blend = clamp01(0.5 - signed_d_fold);
    let base = mix3(PAGE, FOLD, fold_blend);

    // 面纱：从顶部垂下，底边 V 形收尖于 (cx, vpt)。
    let margin = 0.06 * s;
    let (l, t, r) = (margin, margin, s - margin);
    let vpt = 0.60 * s;
    let cx = 0.5 * s;
    let cross_l = (x - l) * (vpt - t) - (y - t) * (cx - l);
    let cross_r = (x - cx) * (t - vpt) - (y - vpt) * (r - cx);
    let veil_inside = cross_l > 0.0 && cross_r > 0.0;
    let d_veil_edge = seg_dist(x, y, l, t, cx, vpt).min(seg_dist(x, y, cx, vpt, r, t));
    let signed_d_veil = if veil_inside { -d_veil_edge } else { d_veil_edge };
    let veil_a = clamp01(0.5 - signed_d_veil) * page_a;

    // 面纱垂直渐变（顶亮底暗，模拟织物受光）。
    let grad_t = clamp01((y - t) / (vpt - t + 0.001));
    let veil_color = mix3(VEIL_LIGHT, VEIL_DARK, grad_t);

    // 文字行：V 形开口下方 3 条横线，代表被揭示的源码。
    let lx_l = cx - 0.22 * s;
    let lx_r = cx + 0.22 * s;
    let lthick = (0.012 * s).max(1.0);
    let mut ink_a = 0.0f32;
    for i in 0..3 {
        let ly = vpt + (0.08 + 0.08 * i as f32) * s;
        let d_line = seg_dist(x, y, lx_l, ly, lx_r, ly) - lthick;
        ink_a = ink_a.max(clamp01(0.5 - d_line));
    }
    ink_a *= page_a * (1.0 - veil_a);

    // 合成：基底 → 面纱 → 墨色文字行。
    let after_veil = mix3(base, veil_color, veil_a);
    let col = mix3(after_veil, INK, ink_a);

    [
        (col[2] * 255.0).round() as u8,
        (col[1] * 255.0).round() as u8,
        (col[0] * 255.0).round() as u8,
        (page_a * 255.0).round() as u8,
    ]
}

/// 文档图标单像素 BGRA：折角纸页 + M↓ 徽标。
fn doc_icon_pixel(x: f32, y: f32, s: f32) -> [u8; 4] {
    let (d_body, signed_d_fold, _) = page_geometry(x, y, s);
    let page_a = clamp01(0.5 - d_body);

    let small = s <= 24.0;
    let fold_blend = if small { 0.0 } else { clamp01(0.5 - signed_d_fold) };
    let base = mix3(PAGE, FOLD, fold_blend);

    // M↓ 徽标，居中于纸页。
    let cx = 0.5 * s;
    let m_top = if small { 0.26 * s } else { 0.30 * s };
    let m_bottom = 0.56 * s;
    let mw = if small { 0.42 * s } else { 0.30 * s };
    let m_left = cx - mw / 2.0;
    let m_right = cx + mw / 2.0;
    let thick = (if small { 0.11 * s } else { 0.07 * s }).max(1.2) / 2.0;

    // M：左竖、左斜、右斜、右竖。
    let d_m = seg_dist(x, y, m_left, m_bottom, m_left, m_top)
        .min(seg_dist(x, y, m_left, m_top, cx, m_bottom))
        .min(seg_dist(x, y, cx, m_bottom, m_right, m_top))
        .min(seg_dist(x, y, m_right, m_top, m_right, m_bottom))
        - thick;

    // ↓ 箭头：竖杆 + 底部三角头。
    let arrow_top = m_bottom + if small { 0.06 * s } else { 0.05 * s };
    let arrow_bottom = if small { 0.76 * s } else { 0.68 * s };
    let aw = mw * 0.45;
    let ah = aw;
    let d_arrow = seg_dist(x, y, cx, arrow_top, cx, arrow_bottom)
        .min(seg_dist(x, y, cx - aw, arrow_bottom - ah, cx, arrow_bottom))
        .min(seg_dist(x, y, cx, arrow_bottom, cx + aw, arrow_bottom - ah))
        - thick;

    let d_symbol = d_m.min(d_arrow);
    let symbol_a = clamp01(0.5 - d_symbol) * page_a;

    let col = mix3(base, INK, symbol_a);

    [
        (col[2] * 255.0).round() as u8,
        (col[1] * 255.0).round() as u8,
        (col[0] * 255.0).round() as u8,
        (page_a * 255.0).round() as u8,
    ]
}

// ---- 工具函数 ----

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// 圆角矩形有符号距离（内部为负）。
fn rounded_rect_sdf(x: f32, y: f32, l: f32, t: f32, r: f32, b: f32, radius: f32) -> f32 {
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let (hx, hy) = ((r - l) / 2.0 - radius, (b - t) / 2.0 - radius);
    let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    outside + dx.max(dy).min(0.0) - radius
}

/// 点到线段距离。
fn seg_dist(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (vx, vy) = (x1 - x0, y1 - y0);
    let t = (((x - x0) * vx + (y - y0) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    ((x - x0 - vx * t).powi(2) + (y - y0 - vy * t).powi(2)).sqrt()
}
