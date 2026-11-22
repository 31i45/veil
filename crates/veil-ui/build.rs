//! 构建身份注入（P0-1）：把 git 短哈希与脏标记编译进 `VEIL_BUILD_COMMIT`。
//!
//! 零新增依赖、不联网：只调用本机 `git`；git 缺失、无 `.git` 或任何命令失败时
//! 注入 `unknown`，构建照常成功（构建身份不是构建前置条件）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest.parent().and_then(Path::parent);

    let commit = root
        .and_then(build_commit)
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VEIL_BUILD_COMMIT={commit}");
}

/// 仓库根目录可用时：`<short hash>` 或 `<short hash>-dirty`；任一步失败返回 None。
fn build_commit(root: &Path) -> Option<String> {
    let head = root.join(".git").join("HEAD");
    if !head.exists() {
        return None;
    }
    println!("cargo:rerun-if-changed={}", head.display());
    let hash = git(root, &["rev-parse", "--short", "HEAD"])?;
    if hash.is_empty() {
        return None;
    }
    let dirty = !git(root, &["status", "--porcelain"])?.is_empty();
    Some(if dirty { format!("{hash}-dirty") } else { hash })
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
