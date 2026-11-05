//! 边界守卫（D3.1）：`veil-core` 必须保持零 GUI / 零异步依赖。
//! 依赖边界是构建事实，直接读被守护 crate 的清单，比文档约定更早失效。
//! 缓存失效入口的唯一性由 `app_tests::memo_hit_equals_forced_after_any_sequence`
//! 等基线性质测试守护（派生缓存命中 == 强制重算）。

const CORE_MANIFEST: &str = include_str!("../../veil-core/Cargo.toml");

/// 取 `[dependencies]` 段（不含 dev-dependencies）。
fn core_dependencies_section() -> &'static str {
    CORE_MANIFEST
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]"))
        .map(|(deps, _)| deps)
        .expect("veil-core/Cargo.toml 必须含 [dependencies] 与 [dev-dependencies] 段")
}

#[test]
fn veil_core_dependencies_stay_minimal() {
    let names: Vec<&str> = core_dependencies_section()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim()))
        .filter(|k| !k.is_empty())
        .collect();
    assert_eq!(
        names,
        ["lapce-xi-rope"],
        "veil-core 依赖面变化需先过 ADR-001（内核不得引入 GUI/异步运行时）"
    );
}

#[test]
fn veil_core_manifest_mentions_no_gui_or_async_crate() {
    for banned in [
        "winit",
        "cosmic-text",
        "softbuffer",
        "arboard",
        "notify",
        "latex-rust",
        "tokio",
        "async-std",
        "pollster",
    ] {
        assert!(
            !CORE_MANIFEST.contains(banned),
            "veil-core 不得依赖 {banned}"
        );
    }
}
