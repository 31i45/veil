//! Mermaid 11.12.0 最小合法图型 corpus。
//!
//! 清单按内嵌 bundle 的 detector 与 QuickJS 实际渲染结果维护；每项只保留能覆盖
//! 对应 parser/layout 的最小源码。这里不包含上游存在、但当前 bundle 未注册的图型。

/// `(稳定名称, Mermaid 源码)`。名称同时用作证据 SVG 文件名。
pub const MERMAID_11_12_CORPUS: &[(&str, &str)] = &[
    ("flowchart", "flowchart TD\n  A[开始] --> B{判断}\n  B -->|是| C[结束]"),
    ("sequence", "sequenceDiagram\n  participant A as Alice\n  participant B as Bob\n  A->>B: Hello\n  B-->>A: Hi"),
    ("class", "classDiagram\n  class Animal {\n    +String name\n    +eat()\n  }\n  class Dog\n  Animal <|-- Dog"),
    ("state", "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Working\n  Working --> [*]"),
    ("er", "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE_ITEM : contains"),
    ("journey", "journey\n  title Working day\n  section Work\n    Make tea: 5: Me\n    Write code: 3: Me"),
    ("gantt", "gantt\n  title Project\n  dateFormat YYYY-MM-DD\n  section Build\n    Design: a1, 2025-01-01, 2d\n    Code: after a1, 3d"),
    ("pie", "pie showData\n  title Languages\n  \"Rust\" : 45\n  \"Go\" : 30\n  \"Python\" : 25"),
    ("quadrant", "quadrantChart\n  title Reach and engagement\n  x-axis Low Reach --> High Reach\n  y-axis Low Engagement --> High Engagement\n  quadrant-1 Expand\n  quadrant-2 Promote\n  quadrant-3 Re-evaluate\n  quadrant-4 Improve\n  Campaign A: [0.3, 0.6]\n  Campaign B: [0.7, 0.8]"),
    ("requirement", "requirementDiagram\n  requirement test_req {\n    id: 1\n    text: the test text.\n    risk: high\n    verifymethod: test\n  }\n  element test_entity {\n    type: simulation\n  }\n  test_entity - satisfies -> test_req"),
    ("gitgraph", "gitGraph\n  commit id: \"one\""),
    ("c4context", "C4Context\n  title System Context\n  Person(customer, \"Customer\", \"A customer\")\n  System(system, \"System\", \"Does work\")\n  Rel(customer, system, \"Uses\")"),
    ("mindmap", "mindmap\n  root((中心))\n    子节点一\n      孙节点\n    子节点二"),
    ("timeline", "timeline\n  title History\n  2024 : First event\n  2025 : Second event"),
    ("sankey", "sankey-beta\n  A,B,5\n  B,C,3\n  A,C,2"),
    ("xychart", "xychart-beta\n  title \"Sales\"\n  x-axis [Jan, Feb, Mar]\n  y-axis \"Revenue\" 0 --> 100\n  bar [20, 40, 60]\n  line [10, 30, 50]"),
    ("block", "block-beta\n  columns 1\n  A[\"A\"]\n  B[\"B\"]\n  A --> B"),
    ("packet", "packet-beta\n  title TCP\n  0-15: \"Source Port\"\n  16-31: \"Destination Port\""),
    ("kanban", "kanban\n  todo\n    id1[Write tests]\n  done\n    id2[Design]"),
    ("architecture", "architecture-beta\n  group api(cloud)[API]\n  service db[Database] in api\n  service server[Server] in api\n  db:L -- R:server"),
    ("radar", "radar-beta\n  title Skills\n  axis A, B, C\n  curve a{1,2,3}\n  max 5\n  min 0"),
    ("treemap", "treemap-beta\n  \"Phones\": 30\n  \"Laptops\": 20"),
];

/// 明确不承诺的图型：当前内嵌 11.12.0 bundle 没有相应 detector。
/// `info` 是 Mermaid 内部诊断图，不作为用户文档图型计入 corpus。
pub const UNSUPPORTED_DIAGRAMS: &[(&str, &str)] = &[
    ("zenuml", "11.12.0 bundle 未内嵌 ZenUML detector"),
    ("wireviz", "外部图型，未向当前 QuickJS 引擎注册"),
    ("venn-beta", "当前 11.12.0 bundle 无此 detector"),
];
