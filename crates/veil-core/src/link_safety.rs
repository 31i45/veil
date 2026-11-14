//! 链接目标安全判定（HTML 导出、编辑器打开链接、剪贴板转换共用）。
//!
//! 文档内容是不可信输入：`javascript:` / `data:` / `file:` / `vbscript:` 等
//! scheme 一旦进入 `href` 或交给系统 shell，就是脚本/本地程序执行入口。
//! 这里只做「提取 scheme + 白名单比较」，归一化（`www.` 补协议等）仍由调用方负责。
//!
//! 浏览器在解析 URL 前会忽略空白/控制字符，并把实体与百分号解码；
//! 判定前先做同样的混淆还原，避免 `java\tscript:`、`javascript&#58;`、
//! `%6a%61vascript:` 之类的变体绕过白名单。

/// 允许出现在链接目标里的显式 scheme（小写）。
const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// 链接目标是否可安全写入 `href`/`src` 或交给系统打开。
///
/// 放行：白名单 scheme、锚点（`#…`）、相对路径、协议相对地址（`//host`）。
/// 拒绝：`javascript:`/`data:`/`file:`/`vbscript:` 及其大小写、实体、百分号、
/// 空白混淆形式，以及其他任何未列出的显式 scheme。
pub fn link_url_is_safe(url: &str) -> bool {
    match link_scheme(url) {
        None => true,
        Some(scheme) => SAFE_SCHEMES.contains(&scheme.as_str()),
    }
}

/// 百分号解码 UTF-8 路径（编辑器本地图片 / HTML 导出资产共用）；非法序列原样保留。
pub fn percent_decode_path(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let b = value.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_val(b.get(i + 1)), hex_val(b.get(i + 2))) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 提取归一化后的 scheme（小写）。相对路径、锚点、协议相对地址返回 `None`。
///
/// 判定规则与浏览器一致：`:` 必须出现在第一个 `/`、`?`、`#` 之前；
/// scheme 形如 `[A-Za-z][A-Za-z0-9+.-]*`。
pub fn link_scheme(url: &str) -> Option<String> {
    let decoded = decode_obfuscation(url);
    // 浏览器解析前会剔除 URL 内的控制字符（含 tab/换行），并去掉首尾空白；
    // 中间空格不剔除——它会让 scheme 非法，浏览器按相对地址处理。
    let compact: String = decoded.chars().filter(|c| !c.is_control()).collect();
    let compact = compact.trim();
    let colon = compact.find(':')?;
    let head = &compact[..colon];
    if head.is_empty() || head.contains('/') || head.contains('?') || head.contains('#') {
        return None;
    }
    if !head.as_bytes()[0].is_ascii_alphabetic()
        || !head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(head.to_ascii_lowercase())
}

/// 最多 4 轮实体/百分号解码：覆盖 `&amp;#58;` 这类二次编码。
fn decode_obfuscation(raw: &str) -> String {
    let mut cur = raw.to_string();
    for _ in 0..4 {
        let next = decode_once(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn decode_once(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some((ch, used)) = decode_entity(&s[i..]) {
                out.push(ch);
                i += used;
                continue;
            }
        }
        if b[i] == b'%' {
            if let (Some(h), Some(l)) = (hex_val(b.get(i + 1)), hex_val(b.get(i + 2))) {
                out.push(char::from(h * 16 + l));
                i += 3;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap_or('\u{0}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn hex_val(b: Option<&u8>) -> Option<u8> {
    let b = *b?;
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 解码 `&#58;` / `&#x3a;` / `&colon;` 等实体；返回 `(字符, 消耗字节数)`。
fn decode_entity(s: &str) -> Option<(char, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'&') {
        return None;
    }
    if b.get(1) == Some(&b'#') {
        let (base, mut i) = match b.get(2) {
            Some(b'x') | Some(b'X') => (16u32, 3usize),
            _ => (10u32, 2usize),
        };
        let start = i;
        while i < b.len() && (b[i] as char).is_digit(base) {
            i += 1;
        }
        if i == start {
            return None;
        }
        let code = u32::from_str_radix(&s[start..i], base).ok()?;
        if b.get(i) == Some(&b';') {
            i += 1;
        }
        return Some((char::from_u32(code)?, i));
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_alphanumeric() {
        i += 1;
    }
    let ch = match s[1..i].to_ascii_lowercase().as_str() {
        "colon" => ':',
        "tab" => '\t',
        "newline" => '\n',
        "sol" => '/',
        "amp" => '&',
        "num" => '#',
        "period" => '.',
        "lowbar" => '_',
        "hyphen" | "dash" => '-',
        _ => return None,
    };
    if b.get(i) == Some(&b';') {
        i += 1;
    }
    Some((ch, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_web_mail_tel_and_local_targets() {
        for ok in [
            "https://example.com/a?b=1#c",
            "HTTP://example.com",
            "  https://example.com",
            "mailto:a@b.c",
            "MAILTO:a@b.c",
            "tel:+8613800000000",
            "#install",
            "img/a.png",
            "./a.md",
            "../a.md",
            "/abs/path",
            "?q=1",
            "//example.com/x",
            "",
        ] {
            assert!(link_url_is_safe(ok), "应放行: {ok:?}");
        }
    }

    #[test]
    fn rejects_dangerous_schemes() {
        for bad in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "  javascript:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "DATA:image/svg+xml,<svg/>",
            "file:///C:/Windows/System32/calc.exe",
            "FILE:///C:/x",
            "vbscript:MsgBox(1)",
            "jar:file:///x",
            "veil-audit://do-not-open",
            "C:\\Windows\\calc.exe",
            "javascript&colon;alert(1)",
            "javascript&#58;alert(1)",
            "javascript&#x3A;alert(1)",
            "java%0ascript:alert(1)",
            "%6a%61vascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            " java&#x9;script:alert(1)",
        ] {
            assert!(!link_url_is_safe(bad), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn interior_space_is_not_a_scheme() {
        // 中间空格使 scheme 非法：浏览器按相对地址处理，不会执行脚本。
        assert_eq!(link_scheme("java script:alert(1)"), None);
        assert!(link_url_is_safe("my file:name.md"));
    }

    #[test]
    fn scheme_extraction_matches_browser_rules() {
        assert_eq!(link_scheme("HTTPS://a.b").as_deref(), Some("https"));
        assert_eq!(link_scheme("tel:+1").as_deref(), Some("tel"));
        assert_eq!(link_scheme("a/b:c"), None, "冒号在 / 之后不是 scheme");
        assert_eq!(link_scheme("?x=1:y"), None);
        assert_eq!(link_scheme("#a:b"), None);
        assert_eq!(link_scheme("1abc:x"), None, "scheme 须以字母开头");
        assert_eq!(link_scheme("a_b:x"), None, "scheme 不允许下划线");
    }

    #[test]
    fn entity_and_percent_decoding_does_not_break_normal_urls() {
        assert_eq!(decode_once("a%20b"), "a b");
        assert_eq!(decode_once("&amp;"), "&");
        assert_eq!(decode_once("100%"), "100%");
        assert_eq!(decode_once("&#65;"), "A");
        assert_eq!(decode_once("&#x41;"), "A");
    }
}
