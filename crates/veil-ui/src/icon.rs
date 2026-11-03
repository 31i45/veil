// 应用图标（单一真源）：深色圆角蓝底 + 白色「V」，纯代码生成、零二进制资产。
// 同一份像素两处复用（本文件不能含 `//!` 内部文档注释：build.rs 以 include! 嵌入）：
// - veil-egui 窗口图标：app.rs 调 `icon_rgba` 喂 winit `with_window_icon`（3e 修 A1，
//   exe 内嵌 ICO 只管任务栏/文件管理器，标题栏图标必须显式设置）；
// - veil build.rs 生成 ICO 文件资源：`include!` 本文件后调 `icon_bytes`。
// 改色/改形只改本文件。

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

/// ICO 容器（16/32/48/64 四档，不压缩）：PNG 压缩需 zlib，故用 BMP-DIB 帧（体积仅 ~22KB）。
/// 仅 build.rs 生成 exe 文件资源时使用；GUI 侧用 `icon_rgba`。
#[allow(dead_code)]
pub fn icon_bytes() -> Vec<u8> {
    const SIZES: [u32; 4] = [16, 32, 48, 64];
    let mut frames = Vec::new();
    for s in SIZES {
        frames.push(icon_frame(s));
    }
    let mut out = Vec::with_capacity(6 + 16 * SIZES.len() + frames.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type=icon
    out.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (s, f) in SIZES.iter().zip(&frames) {
        out.push(*s as u8); // 0x40 = 64 合法；256 才写 0
        out.push(*s as u8);
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
#[allow(dead_code)]
fn icon_frame(s: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((s * s * 4) as usize);
    for y in (0..s).rev() {
        // 自底向上
        for x in 0..s {
            px.extend_from_slice(&icon_pixel(x as f32 + 0.5, y as f32 + 0.5, s as f32));
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

/// 单像素 BGRA：圆角方底色（#3B82F6 蓝）× 白色「V」（两条圆头斜线，按距离抗锯齿）。
fn icon_pixel(x: f32, y: f32, s: f32) -> [u8; 4] {
    let d_bg = rounded_rect_sdf(x, y, 0.06 * s, 0.06 * s, s - 0.06 * s, s - 0.06 * s, 0.22 * s);
    let (t, l, b, r) = (0.26 * s, 0.28 * s, 0.74 * s, 0.72 * s);
    let thick = (0.14 * s).max(1.3) / 2.0;
    let d_v = seg_dist(x, y, l, t, 0.5 * s, b).min(seg_dist(x, y, 0.5 * s, b, r, t)) - thick;
    let bg_a = clamp01(0.5 - d_bg) * 1.0;
    let v_a = clamp01(0.5 - d_v) * bg_a;
    let (bg, v) = ([0x3Bu8, 0x82, 0xF6], [255u8, 255, 255]);
    let mix = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    [
        mix(bg[2], v[2], v_a),
        mix(bg[1], v[1], v_a),
        mix(bg[0], v[0], v_a),
        (bg_a * 255.0).round() as u8,
    ]
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
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
