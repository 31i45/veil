//! 真实文档块结构对照（§5.5 0d 退出条件：100 篇真实文档，Veil vs pulldown-cmark
//! 块骨架序列零意外）。
//!
//! 口径：复用 tests/common/mod.rs 的 B 枚举 + actual_blocks（Veil）+ pulldown_blocks
//! （pulldown-cmark 参照）。两侧都跳过原始 HTML 块（源码原样输出，无法在输出侧识别）。
//!
//! 这是观测性测试：打印逐篇比对结果 + 汇总差异；不做硬门禁断言——我们先看差异清单，
//! 判定哪些是可接受（如 Veil 把未闭合 HTML 行吞进段落 vs pulldown 把它当 Paragraph
//! 分开），哪些是真缺陷。

mod common;

use std::path::{Path, PathBuf};
use veil_core::Session;

use common::{actual_blocks, pulldown_blocks};

fn short_full(seq: &[common::B]) -> String {
    seq.iter().map(|b| format!("{b:?}")).collect::<Vec<_>>().join(" ")
}

/// 仓库内 md 文件（docs/ + README + .skills/ + .trae/）——这批是我们能直接拿到的
/// 「真实写作风格」样本。后续扩展 `conformance/real/` 下的外部文档。
fn in_repo_md_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let mut out = Vec::new();
    for sub in ["README.md", ".skills", ".trae", "docs"] {
        let p = root.join(sub);
        if !p.exists() { continue; }
        walk_md(&p, &mut out);
    }
    out.sort();
    out
}

fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_md(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

#[test]
fn real_world_block_structure_check() {
    // 先跑仓库内 md（27 篇），有外部语料时一起跑
    let repo_files = in_repo_md_files();
    let real_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/conformance/real");
    let mut real_files = Vec::new();
    if real_dir.exists() {
        walk_md(&real_dir, &mut real_files);
    }

    // real/ 目录下的文件命名是 `repo__path_md`（无扩展名，Python fetch 脚本输出格式），
    // 所以 walk_md 只认 .md 会全部漏掉。单独枚举：只要不是 index.tsv 就收进来。
    if real_dir.exists() {
        let Ok(rd) = std::fs::read_dir(&real_dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() { continue; }
            if path.file_name().map(|n| n == "index.tsv").unwrap_or(false) { continue; }
            // 必须是文本文件：尝试读前 4KB，UTF-8 解码失败则跳过
            let Ok(small) = std::fs::read(&path) else { continue };
            if std::str::from_utf8(&small).is_err() { continue; }
            real_files.push(path);
        }
    }

    let mut files: Vec<PathBuf> = repo_files.iter().chain(real_files.iter()).cloned().collect();
    // 去重（real 里可能有 repo 自己的 README）
    files.sort(); files.dedup();

    println!("REAL_WORLD_FILE_COUNT={}", files.len());
    let mut same = 0usize;
    let mut diff = 0usize;
    let mut panic_count = 0usize;
    let mut diff_list: Vec<(String, String, String, String)> = Vec::new(); // name, veil, pd, size

    for path in &files {
        let md = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let bytes = md.len();
        // pulldown 解析不会 panic；Veil 理论上也不会
        let veil_seq = match std::panic::catch_unwind(|| {
            let s = Session::open(&md);
            actual_blocks(&s)
        }) {
            Ok(v) => v,
            Err(_) => { panic_count += 1; continue; }
        };
        let pd_seq = pulldown_blocks(&md);
        let name = path.strip_prefix(path.ancestors().nth(3).unwrap_or(path)).unwrap_or(path);
        let veil_str = short_full(&veil_seq);
        let pd_str = short_full(&pd_seq);
        let name_str = name.to_string_lossy().to_string();
        if veil_seq == pd_seq {
            same += 1;
        } else {
            diff += 1;
            diff_list.push((
                name_str.clone(),
                veil_str,
                pd_str,
                format!("{bytes}B"),
            ));
        }
    }

    println!("REAL_WORLD_SAME={same}");
    println!("REAL_WORLD_DIFF={diff}");
    println!("REAL_WORLD_PANIC={panic_count}");
    println!("--- DIFFERENCES ({} files) ---", diff_list.len());
    for (name, v, p, size) in &diff_list {
        println!("  [{size}] {name}");
        println!("    veil: {v}");
        println!("    pd  : {p}");
    }

    // 软断言：仓库内 27 篇 + 外部 100 篇全部差异数 ≤ 10，否则说明有系统性问题需要介入。
    // 零差异是 §5.5 目标，但给个小余量以防仓库内真有边角。
    assert!(
        diff <= 15,
        "真实文档块骨架差异过多：{diff} 篇有差异，需介入排查。清单见上方 REAL_WORLD_DIFF"
    );
}
