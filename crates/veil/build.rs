//! Windows 版本资源注入（3d 原生打包）：零新增 crate 依赖。
//!
//! `cargo build --release` 产出的单文件 `veil.exe` 即分发产物（无安装器）；
//! 此处只把 `Cargo.toml` 版本与二进制文件属性对齐（INSTANCE §2「版本号一致性」红线：
//! git tag == workspace version == 产物元数据）。资源编译器 rc.exe 取自 Windows SDK，
//! 找不到时**降级跳过**（构建照常成功，仅缺文件属性），不让工具链缺失阻断开发。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 图标像素与 ICO 编码的单一真源在 veil-egui/src/icon.rs（窗口图标与文件资源共用）；
/// build script 不能依赖 workspace crate，故以 include! 复用同一份代码。
#[allow(dead_code)] // icon_rgba 仅 GUI 侧使用
mod icon {
    include!("../veil-egui/src/icon.rs");
}
use icon::icon_bytes;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../veil-egui/src/icon.rs");
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
