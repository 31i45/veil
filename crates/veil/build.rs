//! Windows 版本资源注入（3d 原生打包）：零新增 crate 依赖。
//!
//! `cargo build --release` 产出的单文件 `veil.exe` 即分发产物（无安装器）；
//! 此处只把 `Cargo.toml` 版本与二进制文件属性对齐（INSTANCE §2「版本号一致性」红线：
//! git tag == workspace version == 产物元数据）。资源编译器 rc.exe 取自 Windows SDK，
//! 找不到时**降级跳过**（构建照常成功，仅缺文件属性），不让工具链缺失阻断开发。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RC_EXE");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return; // 仅 MSVC Windows 目标有 VERSIONINFO 资源
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=未找到 rc.exe（Windows SDK），跳过版本资源（产物无文件属性版本）");
        return;
    };

    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let nums = version_numbers(&ver);
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 恒由 cargo 注入"));
    let rc_file = out.join("veil.rc");
    let res_file = out.join("veil.res");
    let ico_file = out.join("veil.ico");
    std::fs::write(&ico_file, icon_bytes()).expect("写 veil.ico");
    std::fs::write(&rc_file, rc_source(&ver, nums)).expect("写 veil.rc");

    let status = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&res_file)
        .arg(format!("/I{}", out.display()))
        .arg(&rc_file)
        .status();
    match status {
        Ok(s) if s.success() && res_file.exists() => {
            println!("cargo:rustc-link-arg={}", res_file.display());
        }
        _ => println!("cargo:warning=rc.exe 编译版本资源失败，跳过（产物无文件属性版本）"),
    }
}

/// "0.1.0" → [0, 1, 0, 0]（VERSIONINFO 要求四段定点数）。
fn version_numbers(ver: &str) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (i, part) in ver.split('.').take(4).enumerate() {
        out[i] = part
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0);
    }
    out
}

fn rc_source(ver: &str, n: [u32; 4]) -> String {
    format!(
        "#pragma code_page(65001)\n\
         1 ICON \"veil.ico\"\n\
         1 VERSIONINFO\n\
         FILEVERSION {a},{b},{c},{d}\n\
         PRODUCTVERSION {a},{b},{c},{d}\n\
         FILEFLAGSMASK 0x3fL\n\
         FILEFLAGS 0x0L\n\
         FILEOS 0x00040004L\n\
         FILETYPE 0x00000001L\n\
         FILESUBTYPE 0x0L\n\
         BEGIN\n\
         \x20 BLOCK \"StringFileInfo\"\n\
         \x20 BEGIN\n\
         \x20   BLOCK \"080404b0\"\n\
         \x20   BEGIN\n\
         \x20     VALUE \"CompanyName\", \"Veil\\0\"\n\
         \x20     VALUE \"FileDescription\", \"Veil Markdown 编辑器\\0\"\n\
         \x20     VALUE \"FileVersion\", \"{ver}\\0\"\n\
         \x20     VALUE \"InternalName\", \"veil\\0\"\n\
         \x20     VALUE \"OriginalFilename\", \"veil.exe\\0\"\n\
         \x20     VALUE \"ProductName\", \"Veil\\0\"\n\
         \x20     VALUE \"ProductVersion\", \"{ver}\\0\"\n\
         \x20   END\n\
         \x20 END\n\
         \x20 BLOCK \"VarFileInfo\"\n\
         \x20 BEGIN\n\
         \x20   VALUE \"Translation\", 0x0804, 1200\n\
         \x20 END\n\
         END\n",
        a = n[0],
        b = n[1],
        c = n[2],
        d = n[3],
    )
}

/// 应用图标（ICO，16/32/48/64 四档，不压缩）：由代码生成——深色圆角底 + 白色 V 字。
/// 不入库二进制资产，改色/改形只改本函数（PNG 压缩图需 zlib，故用 BMP-DIB 帧，体积仅 ~22KB）。
fn icon_bytes() -> Vec<u8> {
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

/// rc.exe 定位：RC_EXE 环境变量 → PATH → Windows SDK（取版本号最大的 x64 目录）。
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RC_EXE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = which("rc.exe") {
        return Some(p);
    }
    let kits = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in std::fs::read_dir(kits).ok()?.flatten() {
        let cand = entry.path().join("x64").join("rc.exe");
        if !cand.is_file() {
            continue;
        }
        let key = numbers(entry.file_name().to_string_lossy().as_ref());
        if best.as_ref().is_none_or(|(k, _)| &key >= k) {
            best = Some((key, cand));
        }
    }
    best.map(|(_, p)| p)
}

fn numbers(s: &str) -> Vec<u64> {
    s.split('.')
        .filter_map(|p| p.parse().ok())
        .collect::<Vec<u64>>()
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}
