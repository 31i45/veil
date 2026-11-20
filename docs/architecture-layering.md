# 三层架构：策略 / 机制 / 编排

> ADR-style 架构守护文档。本文档为 `boundary_guard.rs` 与 crate 顶部层级注释的事实来源。

## 1. 定义

| 层 | 英文名 | 职责 | 特征 |
|---|---|---|---|
| 策略层 | Strategy | 数据模型 + 纯计算 | 无 IO、无状态副作用、可单测 |
| 机制层 | Mechanism | 渲染管线 + 平台桥接 | 有 IO、有缓存、依赖策略层 |
| 编排层 | Orchestration | GUI 主循环 + 用户交互 + 多能力协调 | 依赖策略 + 机制、不自建 GUI 原子 |

## 2. Crate 映射

### 策略层 — veil-core
- `crates/veil-core/` — 整个 crate（唯一策略层 crate）
  - 模块：`buffer`、`callouts`、`history`、`link_safety`、`mermaid`、`range`、`session`、`slug`、`syntax`（block / inline / model）、`toc`
- veil crate 中 **不存在** `core_strategy/` 模块；策略层完整收敛于 `veil-core`。

策略层公开依赖 **仅限 std + 纯计算库**：`lapce-xi-rope`、`flate2`、`base64`、`serde`、`serde_json`。
（扩展的策略层依赖候选：`pulldown-cmark`、`regex`、`latex-rust`、`pdf-writer`、`ttf-parser`、`subsetter`、`notify`、`resvg`——**注意**：`resvg` 同时被机制层使用；策略层只在 PDF 字体子集化时读 resvg 的 `usvg::Tree` 数据，不是光栅化）

**禁止** GUI crate（`winit`、`cosmic-text`、`softbuffer`、`syntect` 在其渲染端）、异步运行时（`tokio`、`async-std`、`pollster`）、网络 IO、QuickJS。

> 现行硬门禁见 `crates/veil-ui/tests/boundary_guard.rs`（`veil_core_dependencies_stay_minimal`、`veil_core_manifest_mentions_no_gui_or_async_crate`）。

### 机制层 — veil-ui + veil-mermaid-offline + veil/platform
- `crates/veil-ui/` — 整个 crate 及所有子模块
  - 顶层模块：`anim`、`app`、`autosave`、`chrome`、`docio`、`document`（doc_state / persist / block_index / recovery / session / snapshot / history）、`editor`、`geometry`、`html`、`i18n`、`image`、`input`、`interaction`、`math`、`mermaid_renderer`、`mode`、`paint`、`perf`、`print`、`search`（含 search_strategy / search_view）、`shell`、`stats`、`style`、`theme`、`tokens`、`view`（cache / ctx_menu / frame / highlight / layout / linebreak / modal / outline / quick_open / settings / style / table / table_strategy / table_view）
- `crates/veil-mermaid-offline/` — 整个 crate（QuickJS + mermaid 引擎 = 渲染机制）
- `crates/veil/src/platform/` — 远程 Mermaid / HTTP / PDF 光栅化 / 进程监视等平台桥接
  - 模块：`assoc`、`clock`、`error`、`http`、`ipc`、`mermaid`、`mermaid_offline`、`pdf`、`pdf_layout`、`pdf_media`、`print`、`shell`、`theme`、`watch`、`wic`、`windows`

机制层公开依赖 **策略层 + 平台库**：`veil-core`、`winit`、`cosmic-text`、`softbuffer`、`syntect`、`image`、`resvg`（光栅化）、WinHTTP、`rquickjs`。

**禁止** veil crate 的编排层类型（GUI 主循环、窗口事件、快捷键）。`veil/ui` 与 `veil-mermaid-offline` 的 Cargo.toml 不得 `path = "../veil"`。

> 现行硬门禁见 `boundary_guard.rs` 中新增的 `veil_ui_does_not_depend_on_veil_crate` 测试。

### 编排层 — veil bin + 顶层协调
- `crates/veil/src/main.rs` — 启动 + wiring（轻量版：远程 Mermaid + resvg 光栅化）
- `crates/veil/src/bin/veil-full.rs` — 完整版启动（QuickJS + mermaid.min.js）
- `crates/veil/src/lib.rs` — 顶层 `run()` 函数 + 多能力协调（编辑循环、预览触发、缓存刷新）
  - 模块：`cli`、`export`、`platform`
- `crates/veil/src/platform/` 虽在 veil crate 内，但其职责属于机制层（平台桥接），因此 **同时** 被机制层与编排层引用是允许的。

依赖 **策略 + 机制** 两层。**禁止** 绕过 `veil-ui` 的 trait 接口直连内部实现（例如不得绕过 `MermaidRenderer` trait 直接调 `resvg::render`）。

## 3. 允许 / 禁止依赖矩阵

| From \\ To | veil-core (策略) | veil-ui (机制) | veil-mermaid-offline (机制) | veil bin (编排) | veil/platform (机制) |
|---|---|---|---|---|---|
| veil-core | ✅ self-ref | ❌ | ❌ | ❌ | ❌ |
| veil-ui | ✅ | ❌ self-ref | ✅ (renderer impl) | ❌ | ✅ (via trait) |
| veil-mermaid-offline | ✅ | ❌ | ❌ self-ref | ❌ | ❌ |
| veil bin | ✅ | ✅ | ✅ | ✅ self-ref | ✅ |
| veil/platform | ✅ | ✅ | ✅ | ❌ | ❌ self-ref |

核心解读：编排层可以引用策略与机制两层；机制层只能下指策略与平行的机制 crate，**绝不能** 上指编排层；策略层对任何上层无感。

## 4. 关键 trait 边界

- `veil_ui::MermaidRenderer` — 机制层对外暴露；编排层不感知内部实现（远程 vs 离线）
- `veil_ui::LatexRenderer` — 同上（纯 Rust vs 外部 WebView2）
- `veil_ui::SyntectHighlighter` — 语法高亮能力
- `veil_ui::Ports` — 编排层传入的依赖集合；所有 UI 子系统通过它拿到策略层与机制层能力

这些 trait 位于 `veil-ui` 顶层 `lib.rs` 的 `pub use` 区，属于 **编排 ↔ 机制** 的契约。任何新增渲染管线都必须走 trait，**不得** 在 `veil/bin` 中直接构造具体实现。

## 5. 与既有架构守卫的关系

- `crates/veil-ui/tests/boundary_guard.rs` 继续生效；本次扩展其检查范围，新增：
  - `veil_ui_does_not_depend_on_veil_crate` — 机制层不得反向依赖编排层
- `veil-core` 顶部注释已声明"零 GUI、零异步运行时依赖"，与 `boundary_guard.rs` 互为表里。
- 各 crate 顶部新增层级注释，使 `cargo doc` 与 `cargo metadata` 即可看出层级归属。
- 未来新增 crate 必须先在本文档登记层级，再提交 `Cargo.toml`，避免边界从一开始就被破坏。
