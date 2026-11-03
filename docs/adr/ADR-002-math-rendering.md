# ADR-002: 公式渲染走 `latex-rust` 纯 Rust 路径；Mermaid 降级为非目标

## 状态：已采纳（2026-10-24 初定为 `rquickjs` 直集成 KaTeX；2026-10-25 修订为 `latex-rust`——KaTeX 只出 HTML，无 webview 像素管线下无法光栅化，属致命阻塞）

## 背景

设计阶段曾提出「KaTeX/Mermaid 提交到 wasmtime 运行」。反证：wasmtime 执行 wasm，不执行 JS；KaTeX 与 Mermaid 均为 JS 库，该路径不自洽。且 tokio 异步运行时对「后台渲染 + 消息回调」这一需求是过度的——std::thread + mpsc 即可覆盖（INSTANCE §6 黑名单已列入 tokio/wasmtime）。

进一步事实：KaTeX 的 `renderToString` 输出 HTML/MathML 字符串，无需 DOM，可在纯 JS 引擎运行；Mermaid 渲染依赖 DOM 布局，headless 路径需要 jsdom/puppeteer 级环境，在 Rust 原生进程内无可行方案。

已核实：`katex` crate 0.4.6 存在（Rust bindings to KaTeX，内嵌 JS 引擎）；`rquickjs` 0.14.0 存在；`latex-rust` 2.1.1 存在（纯 Rust 排版 + 光栅化，字体内嵌；3.0.1 要求 Rust 1.99，当前工具链不可用，锁定 2.1.1）。

## Spike 结果一（2026-10-24，windows-msvc，release 构建，`spikes/katex-timing`）

| 路径 | 结果 |
|---|---|
| `katex` crate 默认 quick-js 后端 | **windows-msvc 上不可编译**：`js_engine.rs` cfg 仅放行 `unix` 或 `windows-gnu`，否则 `compile_error!` |
| `katex` crate duktape 后端（msvc 唯一可编译后端） | **功能不成立**：ducc 0.1.5 内嵌 duktape 2.2.1 无 `Array.prototype.fill`（ES6），内嵌 katex.min.js 渲染 `\begin{pmatrix}` 等矩阵环境即抛 TypeError；crate 不暴露 eval 接口，无法注入 polyfill |
| `rquickjs` 0.14 直集成 vendor katex.min.js | JS 执行层面成立：冷启动 29ms，单公式均值 2.1ms。**但交付物层面不成立，见下方致命阻塞** |

**致命阻塞（2026-10-25 切片 6 启动时确认）**：KaTeX `renderToString` 的产物是 HTML/MathML 字符串，而本项目的呈现管线是 softbuffer 像素直绘、且黑名单禁止一切 webview（INSTANCE §6）。HTML → 位图需要布局引擎级光栅器（webview / 自研 HTML 子集渲染器），前者违禁、后者工作量不可控。即 rquickjs 路径「能跑 JS」但「出不了像素」，否决。

## Spike 结果二（2026-10-25，windows-msvc，release 构建，`spikes/latex-rust-check`，与 katex-timing 同口径 4 类公式 ×100 次）

| 指标 | 实测 |
|---|---|
| 冷启动（字体初始化 + 首渲染） | 1.74ms |
| SVG 渲染均值（text / display） | 211µs / 201µs |
| PNG 渲染均值（text / display，144dpi 透明底） | 584µs / 627µs |
| 100 个公式同步渲染总计 | 21ms（300ms 冷启动预算的 7%） |
| 肉眼质检 | 矩阵 / 求和 / 积分 / 分式根号全部渲染成功，4 张 PNG + 1 SVG 样本目检合格 |

性能判定：单公式 <1ms，100 公式同步渲染仅占预算 7%，**同步渲染 + 块级缓存即可满足 v0**；异步线程 + 按源码 hash 缓存保留为防病态输入的保险丝，取最简形态。

## 决策

1. 公式渲染：`latex-rust`（veil 产品层，锁定 2.1.1，feature `png`），渲染产物 PNG 位图直贴 softbuffer 帧。渲染器以 trait 定义在 veil-ui、实现注入自 veil（与 Clipboard/FileStore 同一注入模式），保持 veil-ui 依赖预算与 headless 可测性。首版采用同步渲染 + 块级缓存；不支持的构造返回 Error 时降级为源码显示，不崩溃。
2. Mermaid：v1 非目标。源码按代码块原样显示，不渲染图。
3. ~~`rquickjs` + vendor katex.min.js~~：HTML 输出在无 webview 像素管线下无法光栅化，否决（本次修订核心）。
4. ~~`katex` crate 主路径~~：windows-msvc 上不可编译 / 功能不成立（见 spike 表一），否决。

## 后果

- 好：零 webview 承诺成立（产品级卖点）；纯 Rust 单 crate，无 JS 引擎、无 vendor 资产维护、无 polyfill 负担；内存占用低于 QuickJS 方案；同步渲染在预算内，首版无需异步架构。
- 坏：latex-rust 成熟度低于 KaTeX——不支持的 LaTeX 构造返回 Error（已定降级策略：显示源码）；数学排版细节（kerning、罕见宏）不及 KaTeX，可接受度由 spike 目检背书；3.x 升级受 Rust 工具链版本制约，锁定 2.1.1 期间需跟踪。

## 替代方案

- `rquickjs` 直集成 vendor katex.min.js：见「致命阻塞」，否决。若未来引入合法光栅化路径可再评估。
- MathJax + resvg 光栅化：MathJax 同为 JS 库需 JS 引擎，且 SVG 光栅化需再引入 resvg，链路长于 latex-rust 单 crate，否决。
- 自研 HTML 子集渲染器：工作量不可控，否决。
- `katex` crate（quick-js/duktape 后端）：windows-msvc 上分别不可编译 / 功能不成立（见 spike 表一），否决。若未来 crate 修复后端覆盖，可再评估换回。
- wasmtime 跑 JS 引擎 wasm 构建：两层虚拟机，复杂度与体积双输，否决。
- webview 渲染块（WebView2）：违反零 webview 黑名单，否决。
- 外部 CLI（如 mermaid-cli）：部署摩擦 + 每次渲染进程冷启动延迟，且仍需 Chromium，否决。
