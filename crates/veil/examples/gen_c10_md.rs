// Generates a 10万字 Markdown file with realistic block structure for C10 testing.
// Run: cargo run --release -p veil --example gen_c10_md
// Then replace the old .txt: move it over `03-10万字.txt` (or keep as .md extension)

use std::fs;
use std::path::Path;

const BLOCK: &str = r#"## 主题 — 深度分析与实践

这里是一段关于某个技术主题的详细讨论。在现代软件开发实践中，我们常常需要面对复杂的系统架构和高性能需求。本章将从历史背景出发，逐步展开各个技术细节的分析与对比，帮助读者建立起完整的认知框架。

### 背景与动机

在进入具体讨论之前，有必要先回顾一下问题的起源。二十世纪九十年代中期以来，互联网应用迅速普及，用户对交互体验的期望水涨船高。与此同时，硬件性能也在以惊人的速度提升，这使得我们能够实现一些过去不可想象的功能。软件产业由此进入了一个新的阶段，工具链和语言生态都在不断演化。

### 核心挑战

在追求极致性能的同时，我们必须平衡以下几个方面，它们之间往往存在张力：

- **启动速度**：冷启动时间应控制在可接受范围内，热启动则需要更低的开销
- **内存占用**：长时间运行不应导致内存持续增长，泄漏必须被及时捕获
- **响应延迟**：用户输入后应即时产生视觉反馈，p95 延迟有明确红线
- **渲染质量**：在不同 DPI、字号、主题下都保持清晰一致的视觉效果
- **可扩展性**：文档规模、窗口尺寸、硬件性能的变化都不应该让系统退化

### 架构设计

整个系统采用三层架构，每一层只依赖其下方的接口。核心层完全不涉及 GUI 或异步运行时，这使得它可以在任何环境中被独立测试和复用，包括单元测试、集成测试、命令行工具和未来可能的 WebAssembly 前端。

| 层 | 职责 | 依赖 |
| --- | --- | --- |
| 产品层 | 菜单 / 快捷键 / 主题 / 生命周期 | 适配层 + winit |
| 适配层 | 事件分发 / 滚动 / 缩放 / 打印 / 光栅 | 核心层 + softbuffer + cosmic-text |
| 核心层 | 解析 / 块结构 / 撤销历史 / 统计 | 零外部依赖 |

```rust
pub struct App<'a> {
    window: winit::window::Window,
    surface: softbuffer::Surface<...>,
    editor: veil_core::Session,
    layout: veil_ui::LayoutEngine,
}
```

> 注意事项：以上数据仅代表某一特定硬件环境下的测量结果，不同机器可能存在差异。建议在目标部署环境中重新跑一遍完整的性能套件，并将结果纳入持续集成的基线监控，这样可以在回归问题出现时第一时间被发现。

### 性能数据

以下数据基于标准基准测试场景，每次测试重复五遍取中位数。测试机器为四核八线程、16GB 内存、NVMe SSD 的标准配置。

- 1,000 块：打开 0.12s，排版 0.21s，击键 p95 2.8ms
- 5,000 块：打开 0.34s，排版 0.67s，击键 p95 5.2ms
- 10,000 块：打开 0.58s，排版 1.12s，击键 p95 9.3ms

### 已知限制

当前版本有一些有意的取舍，它们在未来版本中可能被重新评估。例如，单块渲染上限为六十四 KiB，这为极端情况下的内存放大提供了一个安全网；撤销栈有总字节预算，超出后最旧的撤销步会被淘汰；打印路径目前只支持 Microsoft Print to PDF 驱动，其他驱动的兼容性需要进一步验证。

### 小结

本文综述了一个 Markdown 编辑器从需求到实现的完整路径，涵盖架构、性能、权衡和验证等多个方面。希望这些讨论能为你在类似项目中做出决策时提供参考。真正好的设计不是在纸面上一次成型的，而是在不断迭代和验证中逐步逼近目标的。

"#;

fn main() {
    let out_dir = std::env::var("TEMP").unwrap_or_else(|_| ".".into());
    let out = format!("{}\\veil-walkthrough\\03-10万字.txt", out_dir);
    let path = Path::new(&out);

    let target_chars = 100_000usize;
    let mut buf = String::with_capacity(target_chars * 2);
    let _block_chars = BLOCK.chars().count();
    let mut n = 0;
    loop {
        buf.push_str(BLOCK);
        n += 1;
        if buf.chars().count() >= target_chars {
            break;
        }
    }

    // Trim trailing content that cuts mid-paragraph (best effort)
    // (keeps a little over 100k chars; that's fine for C10)

    fs::write(path, &buf).expect("写入失败");
    let file_size = fs::metadata(path).unwrap().len();
    let content = fs::read_to_string(path).unwrap();
    let total_chars = content.chars().count();
    let lines = content.lines().count();
    let headings = content.matches('#').count();

    println!("路径: {out}");
    println!("大小: {} bytes", file_size);
    println!("字符数: {}", total_chars);
    println!("逻辑行数: {}", lines);
    println!("段落块数（近似）: {}", headings);
    println!("生成了 {} 段完整模板", n);
    println!();
    println!("验证：用 veil 打开后，不应出现「单块渲染上限」降级提示。");
}
