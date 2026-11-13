//! Mermaid v0 工具：pako URL 生成 + 预设模板。
//!
//! mermaid.live 支持 `https://mermaid.live/edit#pako:<compressed>` 格式，
//! 其中 `<compressed>` 是用 deflate 压缩再 base64url 编码的 JSON payload。
//! pako deflate 是 mermaid.live 官方支持的本地解码方式，不经过任何服务器。
//!
//! 约束：本模块不执行任何网络 IO，只生成 URL 字符串供上层消费。
//!
//! 依赖：flate2 v1 已在 crate 依赖树中（miniz_oxide 传递依赖），零体积增量。
//! base64url 编码器手写（~30 行），避免新增第三方 base64 crate。

use flate2::read::GzEncoder;
use flate2::Compression;
use std::io::Read;

/// mermaid.live edit 页 base URL（pako fragment 由本地生成，完全离线）。
pub const MERMAID_LIVE_BASE: &str = "https://mermaid.live/edit#pako:";

/// 流程图模板（flowchart TD）。
pub const FLOWCHART_TEMPLATE: &str = r#"graph TD
    A[Start] --> B{Decision}
    B -->|Yes| C[Action]
    B -->|No| D[End]
    C --> D"#;

/// 时序图模板（sequenceDiagram）。
pub const SEQUENCE_TEMPLATE: &str = "sequenceDiagram\n  participant A as Alice\n  participant B as Bob\n  A->>B: Hello\n  B-->>A: Hi there";

/// 状态图模板（stateDiagram-v2）。
pub const STATE_TEMPLATE: &str =
    "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Working\n  Working --> Done\n  Done --> [*]";

/// ER 图模板（erDiagram）。
pub const ER_TEMPLATE: &str = "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains\n  PRODUCT ||--o{ LINE-ITEM : includes";

/// 类图模板（classDiagram）。
pub const CLASS_TEMPLATE: &str = r#"classDiagram
    class Animal {
        +String name
        +int age
        +eat()
    }
    class Dog {
        +bark()
    }
    Animal <|-- Dog"#;

/// 模板变体枚举，供上层按用户选择取出对应源码。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MermaidTemplate {
    Flowchart,
    Sequence,
    Class,
    State,
    Er,
}

impl MermaidTemplate {
    pub fn source(self) -> &'static str {
        match self {
            Self::Flowchart => FLOWCHART_TEMPLATE,
            Self::Sequence => SEQUENCE_TEMPLATE,
            Self::Class => CLASS_TEMPLATE,
            Self::State => STATE_TEMPLATE,
            Self::Er => ER_TEMPLATE,
        }
    }
}

/// 根据 Mermaid 源码的首个有效行（去空白、跳过 `%%` 注释）识别图表类型，返回 UI 徽标用的短标签。
///
/// 全部匹配基于 `str::starts_with`，零分配。空源码或无法识别时返回 `None`。
pub fn detect_type(source: &str) -> Option<&'static str> {
    let first_line = source
        .lines()
        .map(|l| l.trim_start())
        .find(|l| !l.is_empty() && !l.starts_with("%%"))?;
    match first_line {
        s if s.starts_with("graph ") || s.starts_with("flowchart ") => Some("flowchart"),
        "flowchart-v2" => Some("flowchart"),
        s if s.starts_with("sequenceDiagram") => Some("sequence"),
        "sequenceDiagram-v2" => Some("sequence"),
        s if s.starts_with("classDiagram") => Some("class"),
        s if s.starts_with("stateDiagram") => Some("state"),
        s if s.starts_with("erDiagram") => Some("er"),
        s if s.starts_with("gantt") => Some("gantt"),
        s if s.starts_with("pie") => Some("pie"),
        _ => None,
    }
}

/// 生成 mermaid.live 预览 URL。
///
/// mermaid.live 的 pako payload 是一个压缩后的 JSON，结构如下：
/// ```json
/// {
///   "code": "graph TD\n  A-->B",
///   "mermaid": { "theme": "default" },
///   "autoSync": true,
///   "updateDiagram": true
/// }
/// ```
///
/// 整个 JSON 用 gzip deflate 压缩后 base64url（RFC 4648 §5，无 padding）编码，
/// 拼接到 `https://mermaid.live/edit#pako:` 之后。
///
/// 压缩使用 `flate2::GzEncoder`（gzip 头），mermaid.live 的 pako 解码器可直接识别。
pub fn mermaid_live_url(mermaid_source: &str) -> String {
    let json = format!(
        r#"{{"code":{}, "mermaid":{{"theme":"default"}}, "autoSync":true, "updateDiagram":true}}"#,
        json_escape(mermaid_source)
    );
    let compressed = gzip_bytes(&json);
    let encoded = base64url_encode(&compressed);
    format!("{MERMAID_LIVE_BASE}{encoded}")
}

/// 把任意字符串安全地嵌入 JSON 字符串字面量（无需 serde_json 依赖）。
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Gzip 压缩：flate2 的 GzEncoder 输出标准 gzip 格式。
fn gzip_bytes(input: &str) -> Vec<u8> {
    let mut enc = GzEncoder::new(input.as_bytes(), Compression::default());
    let mut out = Vec::with_capacity(input.len());
    enc.read_to_end(&mut out).expect("flate2 读取不会失败");
    out
}

/// base64url 编码（RFC 4648 §5，无 padding）。
/// 字符集：A-Z a-z 0-9 '-' '_'。手写避免 base64 crate 体积增量。
fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let (full, rem) = bytes.as_chunks::<3>();
    for chunk in full {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
    }
    // 尾部不足 3 字节
    if rem.len() == 1 {
        let n = (rem[0] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
    } else if rem.len() == 2 {
        let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_live_url_starts_with_base() {
        let url = mermaid_live_url("graph TD\n  A-->B");
        assert!(
            url.starts_with(MERMAID_LIVE_BASE),
            "URL 应以 mermaid.live 前缀开头: {url}"
        );
        assert!(url.len() > MERMAID_LIVE_BASE.len());
    }

    #[test]
    fn template_constants_are_nonempty() {
        assert!(!FLOWCHART_TEMPLATE.is_empty());
        assert!(!SEQUENCE_TEMPLATE.is_empty());
        assert!(!CLASS_TEMPLATE.is_empty());
        assert!(!STATE_TEMPLATE.is_empty());
        assert!(!ER_TEMPLATE.is_empty());
        assert!(FLOWCHART_TEMPLATE.contains("graph TD"));
        assert!(SEQUENCE_TEMPLATE.contains("sequenceDiagram"));
        assert!(CLASS_TEMPLATE.contains("classDiagram"));
        assert!(STATE_TEMPLATE.contains("stateDiagram-v2"));
        assert!(ER_TEMPLATE.contains("erDiagram"));
    }

    #[test]
    fn detect_type_known_diagrams() {
        // flowchart
        assert_eq!(detect_type("graph TD\n  A-->B"), Some("flowchart"));
        assert_eq!(detect_type("flowchart TD\n  A-->B"), Some("flowchart"));
        assert_eq!(detect_type("flowchart-v2\n  A-->B"), Some("flowchart"));
        assert_eq!(detect_type("  graph LR\n  A-->B"), Some("flowchart"));
        // sequence
        assert_eq!(
            detect_type("sequenceDiagram\n  A->>B: hi"),
            Some("sequence")
        );
        assert_eq!(
            detect_type("sequenceDiagram-v2\n  A->>B: hi"),
            Some("sequence")
        );
        // class
        assert_eq!(detect_type("classDiagram\n  class A"), Some("class"));
        // state
        assert_eq!(
            detect_type("stateDiagram-v2\n  [*] --> Idle"),
            Some("state")
        );
        assert_eq!(detect_type("stateDiagram\n  [*] --> Idle"), Some("state"));
        // er
        assert_eq!(detect_type("erDiagram\n  A ||--o{ B : rel"), Some("er"));
        // gantt
        assert_eq!(detect_type("gantt\n  title Test"), Some("gantt"));
        // pie
        assert_eq!(detect_type("pie\n  title Pie"), Some("pie"));
        assert_eq!(detect_type("pie title Test\n  A : 1"), Some("pie"));
        // fallback — unknown keyword returns None
        assert_eq!(detect_type("journey\n  title Test"), None);
        assert_eq!(detect_type("mindmap\n  root((Test))"), None);
    }

    #[test]
    fn detect_type_empty_source() {
        assert_eq!(detect_type(""), None);
        assert_eq!(detect_type("   \n\n  "), None);
        // 只有注释行
        assert_eq!(detect_type("%% comment\n%% another\n  %% indented"), None);
        // 注释后没有有效行
        assert_eq!(detect_type("%% comment\n"), None);
        // 注释后才有有效行，应正确识别
        assert_eq!(
            detect_type("%% this is a comment\ngraph TD\n  A-->B"),
            Some("flowchart")
        );
    }

    #[test]
    fn pako_payload_roundtrip() {
        let src = "graph TD\n  A[Hello]-->B[World]";
        let url = mermaid_live_url(src);
        let payload_b64 = url.strip_prefix(MERMAID_LIVE_BASE).unwrap();
        let decoded = base64url_decode(payload_b64);
        let decompressed = inflate_bytes(&decoded);
        let json = String::from_utf8(decompressed).unwrap();
        assert!(
            json.contains("Hello"),
            "解压后的 JSON 应包含原始源码: {json}"
        );
    }

    #[test]
    fn base64url_known_vectors() {
        // RFC 4648 test vectors
        assert_eq!(base64url_encode(b""), "");
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
        assert_eq!(base64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(base64url_encode(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_encode(b"foobar"), "Zm9vYmFy");
        // URL-safe: '+' 和 '/' 应分别变成 '-' 和 '_'
        // 0xFB 0xFF 0xFE → 111110 111111 111111 111110 → '-' '_' '_' '-'
        assert_eq!(base64url_encode(&[0xFB, 0xFF, 0xFE]), "-__-");
    }

    fn base64url_decode(s: &str) -> Vec<u8> {
        const DECODE: [i8; 128] = {
            let mut d = [-1_i8; 128];
            let mut i = 0u8;
            while i < 26 {
                d[(b'A' + i) as usize] = i as i8;
                d[(b'a' + i) as usize] = (i + 26) as i8;
                i += 1;
            }
            let mut i = 0u8;
            while i < 10 {
                d[(b'0' + i) as usize] = (i + 52) as i8;
                i += 1;
            }
            d[b'-' as usize] = 62;
            d[b'_' as usize] = 63;
            d
        };
        let bytes: Vec<u8> = s
            .bytes()
            .filter_map(|b| {
                if b < 128 && DECODE[b as usize] >= 0 {
                    Some(DECODE[b as usize] as u8)
                } else {
                    None
                }
            })
            .collect();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        let (full, rem) = bytes.as_chunks::<4>();
        for c in full {
            let n =
                ((c[0] as u32) << 18) | ((c[1] as u32) << 12) | ((c[2] as u32) << 6) | c[3] as u32;
            out.push(((n >> 16) & 0xFF) as u8);
            out.push(((n >> 8) & 0xFF) as u8);
            out.push((n & 0xFF) as u8);
        }
        match rem.len() {
            2 => {
                let n = ((rem[0] as u32) << 18) | ((rem[1] as u32) << 12);
                out.push(((n >> 16) & 0xFF) as u8);
            }
            3 => {
                let n = ((rem[0] as u32) << 18) | ((rem[1] as u32) << 12) | ((rem[2] as u32) << 6);
                out.push(((n >> 16) & 0xFF) as u8);
                out.push(((n >> 8) & 0xFF) as u8);
            }
            _ => {}
        }
        out
    }

    fn inflate_bytes(bytes: &[u8]) -> Vec<u8> {
        use flate2::read::GzDecoder;
        let mut dec = GzDecoder::new(bytes);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        out
    }
}
