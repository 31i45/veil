//! Mermaid 工具：图型识别与预设模板。
//!
//! 约束：本模块不生成任何外部 URL、不执行网络 IO；图表渲染由
//! `veil-mermaid-offline` 在进程内完成，源码永不离开本机。

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
        s if s.starts_with("journey") => Some("journey"),
        s if s.starts_with("mindmap") => Some("mindmap"),
        s if s.starts_with("gitGraph") => Some("gitGraph"),
        s if s.starts_with("block-beta") => Some("block"),
        s if s.starts_with("requirementDiagram") => Some("requirement"),
        s if s.starts_with("packet-beta") => Some("packet"),
        s if s.starts_with("timeline") => Some("timeline"),
        s if s.starts_with("c4Architecture") => Some("c4"),
        s if s.starts_with("architecture-beta") => Some("architecture"),
        s if s.starts_with("zenuml") => Some("zenuml"),
        s if s.starts_with("wireviz") => Some("wireviz"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // journey / mindmap — 本地引擎同样按首行识别这些图型
        assert_eq!(detect_type("journey\n  title Test"), Some("journey"));
        assert_eq!(detect_type("mindmap\n  root((Test))"), Some("mindmap"));
        // gitGraph / block-beta / requirement 等扩展图型
        assert_eq!(detect_type("gitGraph\n  commit id:\"a\""), Some("gitGraph"));
        assert_eq!(detect_type("block-beta\n  A 1"), Some("block"));
        assert_eq!(
            detect_type("requirementDiagram\n  element A"),
            Some("requirement")
        );
        assert_eq!(detect_type("timeline\n  title Test"), Some("timeline"));
        assert_eq!(detect_type("c4Architecture\n  actor A"), Some("c4"));
        // fallback — 真 unknown keyword 仍返回 None
        assert_eq!(detect_type("not_a_real_diagram\n  foo"), None);
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
}
