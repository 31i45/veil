//! 绘制层颜色字面量静态扫描（防「浅色值写死在绘制层」回归）。
//!
//! 扫描范围：`paint.rs`、`paint_table.rs`、`paint_overlay.rs`、`chrome/`、`view/`
//! 下所有 `.rs`（递归，含 `view/layout/`）。
//!
//! 匹配的颜色字面量（任一命中即失败）：
//! 1. `#RRGGBB` / `#RGB`（含字符串里的 CSS 色值，如 `background-color:#ff0`）；
//! 2. `rgb(` / `rgba(` 函数调用（CSS 风格，`Color::rgb(` 不计入本条）；
//! 3. `Color::rgb(...)` / `Color::rgba(...)` 且**所有实参都是数字字面量**。
//!    实参是变量/表达式时不算颜色字面量：那是 `Theme`/`[u8; 3]` → cosmic-text
//!    `Color` 的唯一转换通道（`view/layout.rs`、`view/layout_engine.rs`），
//!    它只是透传主题色，不引入新的写死色值。
//!
//! 白名单（逐条给出理由，均为窄口径）：
//! - `theme.rs`：调色板的唯一定义处，颜色字面量本来就该写在这里（三主题各自赋值）；
//! - `*_tests.rs` 与 `tests.rs`：测试必须硬编码「期望色」才能断言实现没漂移
//!   （`chrome/tests.rs`、`view/layout_tests.rs`、`view/layout_inline_tests.rs`、
//!   `view/layout/typography_tests.rs`）；
//! - 生产文件内的 `#[cfg(test)]` 区域：这些是文件内联测试模块（`view/style.rs`、
//!   `view/frame.rs`、`view/modal.rs`、`chrome/mod.rs` 等），理由同上，只是文件名
//!   不带 `_tests`；跳过区间从 `#[cfg(test)]` 起，到被修饰项/模块闭合为止；
//! - 行内 `//` 之后：注释里引用色值不是绘制代码（如「沿用浅色 [245,245,245]」）。
//!
//! 失败信息格式：`路径:行号: 命中内容`，按路径排序，便于直接定位。

use std::fs;
use std::path::{Path, PathBuf};

/// veil-ui crate 根目录（编译期注入，测试运行时不依赖当前工作目录）。
fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// 收集扫描目标：三个 paint 文件 + `chrome/`、`view/` 递归下所有 `.rs`。
fn scan_targets() -> Vec<PathBuf> {
    let src = crate_src();
    let mut files = vec![
        src.join("paint.rs"),
        src.join("paint_table.rs"),
        src.join("paint_overlay.rs"),
    ];
    for dir in ["chrome", "view"] {
        collect_rs(&src.join(dir), &mut files);
    }
    files.retain(|p| p.is_file());
    files.sort();
    files
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|ext| ext == "rs") {
            out.push(p);
        }
    }
}

/// 文件名即测试模块：`foo_tests.rs` / `tests.rs`（`#[cfg(test)] mod tests;` 的载体）。
fn is_test_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// 去掉行内注释：`//` 之后不参与扫描（字符串里的 `//` 会一并截断，属可接受误差——
/// 颜色不会写在 URL 之后）。
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// 单行扫描：返回该行命中的字面量片段（注释已剔除）。
fn line_hits(line: &str) -> Vec<String> {
    let code = strip_comment(line);
    let mut hits = Vec::new();
    if let Some(hex) = first_hex(code) {
        hits.push(hex);
    }
    if has_css_call(code, "rgba(") {
        hits.push("rgba(".to_string());
    }
    if has_css_call(code, "rgb(") {
        hits.push("rgb(".to_string());
    }
    if has_literal_ctor(code, "Color::rgba(") || has_literal_ctor(code, "Color::rgb(") {
        hits.push("Color::rgb/rgba(数字字面量)".to_string());
    }
    hits
}

/// 找第一个 `#RRGGBB` / `#RGB`。`#RRGGBBAA`（8 位）与 `#[attr]` 都不算命中。
fn first_hex(code: &str) -> Option<String> {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let start = i + 1;
            let mut n = 0;
            while start + n < b.len() && b[start + n].is_ascii_hexdigit() {
                n += 1;
            }
            if n == 3 || n == 6 {
                return Some(code[i..start + n].to_string());
            }
            // 跳过整段 hex（8 位色不截成 6 位误报；`#[` 走 n=0 分支只前进 1 字节）。
            i = start + n;
            continue;
        }
        i += 1;
    }
    None
}

/// 是否出现独立的 CSS 函数调用（前一个字符不能是标识符字符或 `:`，
/// 避免把 `Color::rgb(` 计入本条）。
fn has_css_call(code: &str, kw: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = code[from..].find(kw) {
        let at = from + pos;
        let prev = code[..at].chars().next_back();
        let standalone =
            !matches!(prev, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == ':');
        if standalone {
            return true;
        }
        from = at + kw.len();
    }
    false
}

/// `Color::rgb(...)` 的实参是否全是数字字面量（变量透传不报警）。
fn has_literal_ctor(code: &str, kw: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = code[from..].find(kw) {
        let args_at = from + pos + kw.len();
        if let Some(args) = balanced_args(&code[args_at..]) {
            if !args.trim().is_empty() && args.split(',').all(|a| is_numeric(a.trim())) {
                return true;
            }
        }
        from = args_at;
    }
    false
}

/// 取到与开括号配对的右括号之间的内容（不含右括号）。
fn balanced_args(s: &str) -> Option<&str> {
    let mut depth = 1i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// 十进制或 `0x` 十六进制字面量（允许 `_` 分隔与 `u8`/`u32` 后缀）。
fn is_numeric(arg: &str) -> bool {
    let a = arg
        .strip_suffix("u8")
        .or_else(|| arg.strip_suffix("u32"))
        .unwrap_or(arg);
    if a.is_empty() {
        return false;
    }
    if let Some(hex) = a.strip_prefix("0x").or_else(|| a.strip_prefix("0X")) {
        return !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit() || c == '_');
    }
    a.chars().all(|c| c.is_ascii_digit() || c == '_')
}

#[test]
fn no_color_literals_in_paint_layers() {
    let targets = scan_targets();
    // 扫描面失效（路径写错/目录被搬走）时必须失败，避免门禁空转。
    assert!(
        targets.len() >= 20,
        "扫描文件过少（{} 个），疑似路径失效: {:?}",
        targets.len(),
        targets
    );
    let mut hits: Vec<String> = Vec::new();
    for path in &targets {
        if is_test_file(path) {
            continue; // 白名单：测试文件的「期望色」。
        }
        let rel = path
            .strip_prefix(crate_src())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "theme.rs" {
            continue; // 白名单：调色板唯一定义处。
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let mut in_test_region = false;
        let mut depth: i32 = 0;
        let mut opened = false;
        for (idx, raw) in text.lines().enumerate() {
            let code = strip_comment(raw);
            if in_test_region {
                // 粗略按花括号闭合判断区间结束；内联测试模块在文件尾部，花括号
                // 不会跨出字符串字面量（本仓库测试代码无此写法）。
                depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
                if depth > 0 {
                    opened = true;
                }
                let closed = (opened && depth <= 0) || (!opened && code.trim_end().ends_with(';'));
                if closed {
                    in_test_region = false;
                    depth = 0;
                    opened = false;
                }
                continue;
            }
            if code.trim_start().starts_with("#[cfg(test)]") {
                in_test_region = true;
                depth = 0;
                opened = false;
                continue; // 白名单：文件内联测试区域。
            }
            for hit in line_hits(raw) {
                hits.push(format!("{rel}:{}: {}  [命中 {hit}]", idx + 1, raw.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "绘制层出现颜色字面量（应改取 Theme 字段；确需新语义色就在 theme.rs 新增字段）：\n{}",
        hits.join("\n")
    );
}

/// 扫描器自检：确保规则既能抓到字面量，也不会把变量透传/注释/属性误报成违规。
/// 没有这条，「零命中」可能只是规则失效（门禁空转）。
#[test]
fn scanner_flags_literals_and_ignores_theme_passthrough() {
    let flagged = [
        ("fill_rect(0, 0, 1, 1, [255, 255, 255, 255]);", false),
        (r#"let css = "background-color:#ff0";"#, true),
        ("let c = #1e1e1e;", true),
        ("let c = Color::rgb(255, 0, 0);", true),
        ("let c = Color::rgba(0x1e, 0x1e, 0x1e, 255);", true),
        (r#"let c = "rgb(12, 24, 36)";"#, true),
        (r#"let c = "rgba(12, 24, 36, 0.5)";"#, true),
    ];
    for (line, expect) in flagged {
        assert_eq!(
            !line_hits(line).is_empty(),
            expect,
            "扫描器对 `{line}` 的判定不符合预期"
        );
    }
    let clean = [
        "let c = Color::rgb(r, g, b);",
        "let c = Color::rgb(base[0], base[1], base[2]);",
        "// 沿用浅色 #f5f5f5 会造成眩光",
        "#[cfg(test)]",
        "#[allow(clippy::too_many_arguments)]",
        "frame.fill_rect(x, y, w, h, theme.kbd_bg);",
        "let alpha = 255;",
        "let id = 0x1234u32;",
    ];
    for line in clean {
        assert!(
            line_hits(line).is_empty(),
            "`{line}` 不应被判定为颜色字面量: {:?}",
            line_hits(line)
        );
    }
}
