//! 边界守卫（D3.1）：`veil-core` 必须保持零 GUI / 零异步依赖。
//! 依赖边界是构建事实，直接读被守护 crate 的清单，比文档约定更早失效。
//! 缓存失效入口的唯一性由 `app_tests::memo_hit_equals_forced_after_any_sequence`
//! 等基线性质测试守护（派生缓存命中 == 强制重算）。

const CORE_MANIFEST: &str = include_str!("../../veil-core/Cargo.toml");

/// 策略层依赖白名单：veil-core 只允许这三项。
/// 新增依赖须同步更新 ALLOWED 并说明理由（ADR-001：内核不得引入 GUI/异步运行时）。
const ALLOWED: [&str; 3] = ["lapce-xi-rope", "serde", "serde_json"];

/// GUI / 异步运行时黑名单（不区分连字符/下划线写法）。
const BANNED_CRATES: [&str; 9] = [
    "winit",
    "cosmic-text",
    "softbuffer",
    "arboard",
    "notify",
    "latex-rust",
    "tokio",
    "async-std",
    "pollster",
];

/// 取 `[dependencies]` 段（不含 dev-dependencies）。
fn core_dependencies_section() -> &'static str {
    CORE_MANIFEST
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]"))
        .map(|(deps, _)| deps)
        .expect("veil-core/Cargo.toml 必须含 [dependencies] 与 [dev-dependencies] 段")
}

/// `[dependencies]` 段内出现的顶层键（依赖名）。
fn core_dependency_names() -> Vec<&'static str> {
    core_dependencies_section()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim()))
        .filter(|k| !k.is_empty())
        .collect()
}

/// 白名单语义：出现未登记依赖即失败（精确清单匹配会在条目顺序/注释变化时误报）。
#[test]
fn veil_core_dependencies_stay_minimal() {
    let unknown: Vec<&str> = core_dependency_names()
        .into_iter()
        .filter(|k| !ALLOWED.contains(k))
        .collect();
    assert!(
        unknown.is_empty(),
        "veil-core 出现未登记的依赖 {unknown:?}：新增依赖须同步更新 ALLOWED 并说明理由\
         （ADR-001：内核不得引入 GUI/异步运行时）"
    );
}

/// 白名单自身不得收录 GUI / 异步黑名单里的 crate（防止门禁被放宽）。
#[test]
fn allowed_core_dependencies_exclude_gui_and_async_crates() {
    for banned in BANNED_CRATES {
        let hyphen = banned.replace('_', "-");
        let underscore = banned.replace('-', "_");
        assert!(
            !ALLOWED
                .iter()
                .any(|a| a.replace('_', "-") == hyphen || a.replace('-', "_") == underscore),
            "ALLOWED 不得收录 {banned}"
        );
    }
}

#[test]
fn veil_core_manifest_mentions_no_gui_or_async_crate() {
    for banned in BANNED_CRATES {
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

/// 机制层 crate 之间不得反向依赖：veil-mermaid-offline 不引用 veil-ui。
#[test]
fn veil_mermaid_offline_does_not_depend_on_veil_ui() {
    let offline_toml = include_str!("../../veil-mermaid-offline/Cargo.toml");
    assert!(
        !offline_toml.contains("veil-ui"),
        "veil-mermaid-offline（机制层）不得反向依赖 veil-ui"
    );
}

// ---------------------------------------------------------------------------
// 编排层入口守卫：不得绕过 MermaidRenderer trait 直连 SVG 光栅化实现
// ---------------------------------------------------------------------------

/// 编排层（`veil/src/lib.rs`、`veil/src/export.rs`）只能经 `MermaidRenderer` trait
/// 消费 Mermaid；出现 resvg/usvg 即意味着绕过 trait 接缝直连光栅化实现。
/// `latex_rust` 不在禁止之列：编排层用它构造 `MathRenderer` 实现（ADR-002）。
#[test]
fn orchestration_layer_does_not_rasterize_svg_directly() {
    for (name, src) in [
        ("veil/src/lib.rs", include_str!("../../veil/src/lib.rs")),
        (
            "veil/src/export.rs",
            include_str!("../../veil/src/export.rs"),
        ),
    ] {
        for needle in ["resvg::", "usvg::"] {
            assert!(
                !src.contains(needle),
                "编排层 {name} 不得直连光栅化实现（出现 `{needle}`）：请经 MermaidRenderer trait 调用"
            );
        }
    }
}
