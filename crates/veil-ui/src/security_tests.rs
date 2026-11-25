//! 威胁模型回归测试（raise-veil-quality-ceiling / Task 6 填充）。
//!
//! 每个测试对应 `SECURITY.md`「威胁模型」一节的一类攻击面：构造最小恶意输入，
//! 断言输出被中和/拒绝（而非「不 panic」）。
//!
//! 说明：完整导出器的成品断言位于产品层 `crates/veil/src/export.rs::tests`
//! （本 crate 无法调用 `HtmlExporter`）。这里锁定导出与预览共同依赖的共享过滤原语
//! （`veil_core::should_strip_html` / `sanitize_style` / `link_url_is_safe` 与
//! `veil-ui::url_is_openable`），它们是导出成品净化的实际执行者。

use std::path::{Path, PathBuf};

use veil_core::{link_url_is_safe, sanitize_style, should_strip_html, HTML_INLINE_WHITELIST};

use crate::html::html_to_markdown;

// ---- 1. HTML 注入：Markdown 原始 HTML 的标签级过滤 ----

/// 危险标签必须整段剥除，且不得出现在行内白名单里；大小写不得绕过。
#[test]
fn raw_html_dangerous_tags_are_stripped_not_escaped_through() {
    for tag in [
        "script", "style", "iframe", "object", "embed", "form", "textarea",
    ] {
        assert!(should_strip_html(tag), "{tag} 必须整段剥除");
        assert!(
            should_strip_html(&tag.to_ascii_uppercase()),
            "{tag} 的大小写变体不得绕过"
        );
        assert!(
            !HTML_INLINE_WHITELIST.contains(&tag),
            "{tag} 不得进入行内白名单"
        );
    }
    // 白名单只放行无脚本语义的排版标签
    for tag in ["svg", "math", "video", "audio", "marquee"] {
        assert!(
            !HTML_INLINE_WHITELIST.contains(&tag),
            "{tag} 不得进入行内白名单"
        );
    }
    assert!(HTML_INLINE_WHITELIST.contains(&"kbd"));
}

// ---- 4. 导出 HTML 的脚本注入：URL 与 style 过滤 ----

/// 导出成品会被浏览器打开：URL 白名单必须挡住 `javascript:`/`data:`/`file:` 及其
/// 实体、百分号、控制字符混淆形式；`style` 属性不得承载 `url()`/`expression` 等载体。
#[test]
fn export_url_and_style_filters_block_script_vectors() {
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        " javascript:alert(1)",
        "javascript&colon;alert(1)",
        "javascript&#58;alert(1)",
        "javascript&#x3A;alert(1)",
        "java%0ascript:alert(1)",
        "%6a%61vascript:alert(1)",
        "java\tscript:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "file:///C:/Windows/System32/calc.exe",
        "vbscript:MsgBox(1)",
    ] {
        assert!(!link_url_is_safe(bad), "链接目标应被拒绝: {bad:?}");
        assert!(!crate::url_is_openable(bad), "不得交给系统 shell: {bad:?}");
    }
    for ok in [
        "https://example.com/a",
        "mailto:a@b.c",
        "#anchor",
        "img/a.png",
    ] {
        assert!(link_url_is_safe(ok), "正常目标应放行: {ok:?}");
    }
    assert!(crate::url_is_openable("https://example.com"));

    // style 只保留白名单声明；危险片段整条丢弃。
    let style = sanitize_style(
        "color:red;position:fixed;z-index:9;\
         background-image:url(javascript:alert(1));expression(alert(2))",
    )
    .expect("合法声明 color:red 应保留");
    assert_eq!(style, "color:red");
    for bad in ["javascript:", "position", "z-index", "url(", "expression"] {
        assert!(!style.contains(bad), "style 不得残留 {bad}: {style}");
    }
    assert_eq!(
        sanitize_style("color:expression(alert(1))"),
        None,
        "值里出现危险片段时整条声明丢弃"
    );
}

// ---- 2. `file://` 图片：只接受文件路径，不接受带 scheme 的 URL ----

/// 本地图片加载的入口判定：`file:`/`http(s):`/`data:`/`javascript:` 一律不当本地文件读取。
#[test]
fn file_scheme_images_are_never_loaded() {
    let doc = Path::new("C:/docs/notes.md");
    for url in [
        "file:///C:/Windows/win.ini",
        "FILE:///C:/Windows/win.ini",
        "file://server/share/a.png",
        "http://127.0.0.1/a.png",
        "https://example.com/a.png",
        "data:image/png;base64,iVBORw0KGgo=",
        "javascript:alert(1)",
    ] {
        assert_eq!(
            crate::paint::local_image_path(url, Some(doc)),
            None,
            "{url} 不得按本地文件读取"
        );
    }
    assert!(!crate::paint::is_remote_image_url("file:///C:/a.png"));
    assert!(crate::paint::is_remote_image_url(
        "https://example.com/a.png"
    ));

    // 相对路径以文档目录为基准，百分号编码先解码。
    assert_eq!(
        crate::paint::local_image_path("img/a%20b.png", Some(doc)),
        Some(PathBuf::from("C:/docs/img/a b.png"))
    );
    assert_eq!(
        crate::paint::local_image_path("img/a.png", None),
        None,
        "草稿没有解析基准"
    );

    // 已知放宽（有意行为）：无 scheme 的绝对路径按本地图片加载，便于作者引用本机图片。
    let abs = if cfg!(windows) {
        "C:\\pics\\a.png"
    } else {
        "/pics/a.png"
    };
    assert_eq!(
        crate::paint::local_image_path(abs, None),
        Some(PathBuf::from(abs))
    );
}

// ---- 3. 路径穿越：图片引用侧目录包含校验 ----

/// 基础穿越：`../` 直接跳到文档目录父级以上 → 必须返回 None。
#[test]
fn image_reference_dotdot_is_contained_to_document_dir() {
    let doc = Path::new("C:/docs/notes.md");
    assert_eq!(
        crate::paint::local_image_path("../secret.png", Some(doc)),
        None,
        "直接 `../` 应被拒绝"
    );
    assert_eq!(
        crate::paint::local_image_path("../../secret.png", Some(doc)),
        None,
        "多级 `../../` 应被拒绝"
    );
    assert_eq!(
        crate::paint::local_image_path("img/../../secret.png", Some(doc)),
        None,
        "先下后上的 `img/../../` 应被拒绝"
    );
}

/// 百分号编码穿越：`%2E%2E` 解码后即 `..`，应同一条校验命中。
#[test]
fn image_reference_percent_encoded_dotdot_is_rejected() {
    let doc = Path::new("C:/docs/notes.md");
    assert_eq!(
        crate::paint::local_image_path("%2E%2E/secret.png", Some(doc)),
        None,
        "`%2E%2E` 应解码为 `..` 并越出目录"
    );
    assert_eq!(
        crate::paint::local_image_path("img/%2E%2E/%2E%2E/secret.png", Some(doc)),
        None,
        "混合编码应被拒绝"
    );
}

/// 绝对路径越界：文档在 `D:\docs\notes.md`，绝对路径 `C:\Users\x.png` 跨驱动器 → None。
#[test]
fn image_reference_absolute_outside_document_dir_is_rejected() {
    let doc = Path::new("D:/docs/notes.md");
    let outside = if cfg!(windows) {
        "C:\\Users\\x.png"
    } else {
        "/etc/passwd"
    };
    assert_eq!(
        crate::paint::local_image_path(outside, Some(doc)),
        None,
        "文档目录外的绝对路径应被拒绝：{outside}"
    );
}

/// 合法路径白名单：文档目录内的相对引用与同驱动器绝对引用应正常返回。
#[test]
fn image_reference_legitimate_paths_still_work() {
    let doc = Path::new("C:/docs/notes.md");
    // 相对路径
    let resolved = crate::paint::local_image_path("img/photo.png", Some(doc))
        .expect("文档内相对路径应返回 Some");
    assert_eq!(
        resolved.components().count(),
        Path::new("C:/docs/img/photo.png").components().count(),
        "长度与目标一致"
    );
    assert!(
        resolved.ends_with("img/photo.png") || resolved.ends_with("img\\photo.png"),
        "目标文件名应保留"
    );

    // `./` 前缀
    assert!(
        crate::paint::local_image_path("./photo.png", Some(doc)).is_some(),
        "`./` 前缀不应被误拒绝"
    );

    // 同驱动器、文档目录内的绝对路径
    let inside_abs = if cfg!(windows) {
        "C:\\docs\\assets\\cover.png"
    } else {
        "/docs/assets/cover.png"
    };
    assert_eq!(
        crate::paint::local_image_path(inside_abs, Some(doc)),
        Some(PathBuf::from(inside_abs))
    );
}

// ---- 5. 剪贴板粘贴：HTML → Markdown 的清洗 ----

/// 粘贴通道：危险标签内容、事件属性与危险图片协议不得进入 Markdown。
#[test]
fn pasted_html_events_scripts_and_unsafe_images_are_neutralized() {
    let md = html_to_markdown(
        "<html><head><style>body{display:none}</style></head><body>\
         <script>alert(1)</script>\
         <p>正文 <img src=\"a.png\" alt=\"图\" onerror=\"alert(2)\"></p></body></html>",
    );
    assert_eq!(md, "正文 ![图](a.png)\n");
    for bad in ["<script", "display:none", "onerror"] {
        assert!(!md.contains(bad), "输出不得含 {bad}: {md}");
    }

    // 危险图片协议只保留 alt 文字
    assert_eq!(
        html_to_markdown("<p><img src=\"javascript:alert(1)\" alt=\"图\"></p>"),
        "图\n"
    );
    // 未知标签（含带事件的 svg）不会把属性带到输出
    assert_eq!(html_to_markdown("<svg onload=\"alert(1)\"></svg>"), "");
}

/// 粘贴通道：`javascript:`/`data:`/`file:` 链接降级为纯文本，目标不得进入文档。
#[test]
fn pasted_dangerous_links_are_degraded_to_text() {
    for href in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "java&#115;cript:alert(1)",
        "java%0ascript:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "file:///C:/Windows/notepad.exe",
        "vbscript:MsgBox(1)",
    ] {
        let src = format!("<p><a href=\"{href}\">点我</a></p>");
        let md = html_to_markdown(&src);
        assert_eq!(md, "点我\n", "{href} 应只保留文字");
        assert!(
            !md.to_ascii_lowercase().contains("javascript"),
            "危险 scheme 不得残留: {md}"
        );
    }
    // 白名单内的链接仍按 Markdown 保留
    assert_eq!(
        html_to_markdown("<p><a href=\"https://example.com/x\">示例</a></p>"),
        "[示例](https://example.com/x)\n"
    );
}
