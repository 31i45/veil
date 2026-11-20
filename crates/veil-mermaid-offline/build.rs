//! 构建期把 linkedom 的 ESM 产物转成可直接 eval 的经典脚本。
//!
//! 为什么需要转换：QuickJS 不走模块加载器，且 linkedom 产物顶层有
//! `const window = new WeakMap` 之类的词法声明——直接当经典脚本执行会
//! 遮蔽真正的全局 `window`。包进 IIFE 后词法声明被函数作用域隔离。

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src_path = manifest.join("assets/linkedom.worker.mjs");
    println!("cargo:rerun-if-changed={}", src_path.display());
    println!("cargo:rerun-if-changed=assets/bootstrap.js");
    println!("cargo:rerun-if-changed=assets/mermaid.min.js");

    let src = std::fs::read_to_string(&src_path).expect("read linkedom.worker.mjs");
    let out = esm_to_script(&src);
    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("linkedom.js");
    std::fs::write(out_path, out).expect("write linkedom.js");
}

/// ESM → IIFE 经典脚本，末尾 `export { A, B as C }` 变成 `globalThis.__linkedom = {...}`。
fn esm_to_script(src: &str) -> String {
    let src = &src
        .replace("\nexport const ", "\nconst ")
        .replace("\nexport function ", "\nfunction ")
        .replace("\nexport class ", "\nclass ");

    let idx = src
        .rfind("export {")
        .expect("linkedom: export block not found");
    let head = &src[..idx];
    let tail = &src[idx..];
    let inner = tail
        .trim_start_matches("export {")
        .trim()
        .trim_end_matches(';')
        .trim()
        .trim_end_matches('}')
        .trim();

    let pairs: Vec<String> = inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            match item.split_once(" as ") {
                Some((a, b)) => format!("{}: {}", b.trim(), a.trim()),
                None => format!("{item}: {item}"),
            }
        })
        .collect();

    format!(
        "(function(){{\n{head}globalThis.__linkedom = {{ {} }};\n}})();\n",
        pairs.join(", ")
    )
}
