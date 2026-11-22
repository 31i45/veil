//! 发布文档措辞守护：隐私 / 安全 / 支持文档与双 README 的存在性及 NOT RUN 披露边界。
//!
//! 读取工作区根目录的 Markdown 文件，不依赖任何运行时能力；第三方许可清单
//! 只校验关键组件名，完整性与新鲜度由 `tools/third_party_notices.ps1 -Check` 负责。

use std::path::{Path, PathBuf};

/// 工作区根目录：`crates/veil` 上溯两级。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR 应位于 crates/veil")
        .to_path_buf()
}

/// 读取根目录文档；缺失或不可读时给出可整改的失败信息。
fn read_doc(root: &Path, name: &str) -> String {
    let path = root.join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("缺少或无法读取必备文档 `{name}`（{}）：{e}", path.display()))
}

/// 五个必备文档：隐私、安全、支持与双 README。
const REQUIRED_DOCS: [&str; 5] = [
    "PRIVACY.md",
    "SECURITY.md",
    "SUPPORT.md",
    "README.md",
    "README.zh-CN.md",
];

/// 禁止出现的结论性断言（大小写不敏感）。
const FORBIDDEN_CLAIMS: [&str; 3] = ["已验证零出站", "zero outbound verified", "已验证零业务出站"];

#[test]
fn docs_policy_required_docs_exist_and_are_non_empty() {
    let root = workspace_root();
    let mut problems = Vec::new();

    for name in REQUIRED_DOCS {
        let path = root.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => {
                problems.push(format!("`{name}` 存在但内容为空"));
            }
            Ok(_) => {}
            Err(e) => problems.push(format!(
                "缺少或无法读取 `{name}`（{}）：{e}",
                path.display()
            )),
        }
    }

    assert!(
        problems.is_empty(),
        "必备文档检查未通过：\n- {}",
        problems.join("\n- ")
    );
}

#[test]
fn docs_policy_forbidden_claims_are_absent() {
    let root = workspace_root();
    let mut violations = Vec::new();

    for name in REQUIRED_DOCS {
        let text = read_doc(&root, name);
        let lowered = text.to_lowercase();
        for phrase in FORBIDDEN_CLAIMS {
            if lowered.contains(&phrase.to_lowercase()) {
                violations.push(format!("`{name}` 含禁止短语「{phrase}」"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "文档不得出现「已实测零出站」类结论性断言；请改为「设计上不包含遥测/后台更新/后台请求」并注明抓包验证未执行：\n- {}",
        violations.join("\n- ")
    );
}

#[test]
fn docs_policy_privacy_lists_four_not_run_items() {
    let root = workspace_root();
    let privacy = read_doc(&root, "PRIVACY.md");

    // 四个外部验证关键词必须同时出现，且处于明确的 NOT RUN 语境。
    let keywords = [
        ("生产签名", "生产签名"),
        ("SmartScreen", "SmartScreen"),
        ("抓包", "真实网络抓包"),
        ("Actions", "本提交远端 GitHub Actions"),
    ];

    let mut missing = Vec::new();
    for (needle, label) in keywords {
        if !privacy.contains(needle) {
            missing.push(format!("`PRIVACY.md` 缺少 `{needle}`（{label}）"));
        }
    }
    if !privacy.contains("NOT RUN") && !privacy.contains("NOT\u{00A0}RUN") {
        missing.push("`PRIVACY.md` 缺少 `NOT RUN` 标记".to_string());
    }

    assert!(
        missing.is_empty(),
        "隐私文档必须如实列出四项外部 NOT RUN（生产签名、真实 SmartScreen、真实抓包、本提交远端 Actions）：\n- {}",
        missing.join("\n- ")
    );
}

#[test]
fn docs_policy_third_party_licenses_cover_bundled_assets() {
    let root = workspace_root();
    let text = read_doc(&root, "THIRD-PARTY-LICENSES.md").to_lowercase();
    let mut missing = Vec::new();
    // 内嵌资产与主要依赖：egui 不在依赖图内（零 GUI 框架），故不要求出现。
    for needle in ["mermaid", "quickjs", "linkedom"] {
        if !text.contains(needle) {
            missing.push(format!("`THIRD-PARTY-LICENSES.md` 缺少 `{needle}`"));
        }
    }
    assert!(
        missing.is_empty(),
        "第三方许可清单必须覆盖内嵌资产与主要依赖（大小写不敏感）：\n- {}",
        missing.join("\n- ")
    );
}

#[test]
fn docs_policy_readmes_link_policy_docs() {
    let root = workspace_root();
    let targets = [
        "PRIVACY.md",
        "SECURITY.md",
        "SUPPORT.md",
        "THIRD-PARTY-LICENSES.md",
    ];

    let mut problems = Vec::new();
    for readme in ["README.md", "README.zh-CN.md"] {
        let text = read_doc(&root, readme);
        for target in targets {
            // 只校验链接文本；文件内容与新鲜度由许可清单测试与发布门禁负责。
            let link = format!("]({target})");
            if !text.contains(&link) {
                problems.push(format!("`{readme}` 缺少指向 `{target}` 的链接 `{link}`"));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "双 README 必须提供隐私、安全、支持与第三方许可入口：\n- {}",
        problems.join("\n- ")
    );
}
