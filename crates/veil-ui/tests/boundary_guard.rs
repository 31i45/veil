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
        ["lapce-xi-rope", "flate2", "base64", "serde", "serde_json"],
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

// ---------------------------------------------------------------------------
// 三层架构守卫：机制层不得反向依赖编排层（详见 docs/architecture-layering.md）
// ---------------------------------------------------------------------------

/// veil-ui（机制层）不得引用 veil crate（编排层）的顶层类型。
/// cargo tree -p veil-ui -e no-dev 不应出现 veil crate；此处 cargo test 的
/// 编译阶段已足够保证依赖方向正确，额外 grep Cargo.toml 作为早期硬门禁。
#[test]
fn veil_ui_does_not_depend_on_veil_crate() {
    let veil_ui_toml = include_str!("../../veil-ui/Cargo.toml");
    assert!(
        !veil_ui_toml.contains("veil = ") || !veil_ui_toml.contains("path = \"../veil\""),
        "veil-ui must not depend on veil crate (策略层: veil-core, 编排层: veil bin)"
    );
}

/// veil-mermaid-offline（机制层）同样不得引用 veil crate（编排层）。
#[test]
fn veil_mermaid_offline_does_not_depend_on_veil_crate() {
    let offline_toml = include_str!("../../veil-mermaid-offline/Cargo.toml");
    assert!(
        !offline_toml.contains("veil = ") || !offline_toml.contains("path = \"../veil\""),
        "veil-mermaid-offline must not depend on veil crate (策略层: veil-core, 编排层: veil bin)"
    );
}
