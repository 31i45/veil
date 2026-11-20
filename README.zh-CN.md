# Veil

![Veil](icons/veil.ico)

[![CI](https://github.com/31i45/veil/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/31i45/veil/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/31i45/veil?label=release)](https://github.com/31i45/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable%20%C2%B7%20edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%2011%20%C2%B7%20x64-0078d4.svg)](https://github.com/31i45/veil/releases)
[![Portable](https://img.shields.io/badge/single%20exe%20%C2%B7%20no%20installer%20%C2%B7%20no%20bg%20networking-2ea44f.svg)](https://github.com/31i45/veil/releases)

[English](README.md)

## Veil 的起源

Veil 的灵感来自 Typedown — Typedown 是我最喜欢的 Windows Markdown 编辑器，没有之一，但是它启动需要几十秒到数分钟，每次都让人崩溃。Typedown 是 WinUI 框架实现的应用，冷启动快过你完成一杯咖啡的时间本该是理所当然的事，但现实并非如此。

所以，我决定自己复刻。纯 Rust 实现，无 webview、无 Chromium/WebView2 进程、无外部运行时依赖。单文件 exe，应用内冷启动 ≈190 ms（首次打开并编辑约再增 220 ms；双指标与测量命令/日期见[数字红线](#数字红线目标值与实测)）。**单机单用户 · 零后台网络 · 永不协作**。

## 下载与使用

从 [Releases](https://github.com/31i45/veil/releases) 下载 `veil.exe`。无需安装、无外部运行时依赖、不需要管理员权限。

1. 把 `veil.exe` 放到一个打算长期保留的目录（例如 `%LOCALAPPDATA%\Veil`）。Windows 按路径记录文件关联，之后移动 exe 会让关联失效。
2. 双击运行，或执行 `veil.exe 某个文件.md`。

## 把 .md 文件关联到 Veil

Veil 保持便携：启动路径从不碰注册表。关联需要显式开关，且只写当前用户：

```powershell
veil.exe /register     # 写入当前用户 ProgID + 内嵌文档图标，加入「打开方式」，只填空位
veil.exe /unregister   # 只撤销 /register 创建、且当前值仍属于自己的项
```

应用图标（白页 + 面纱垂坠 V）与 `.md` 文档图标（折页 + M↓）均为 Veil 自绘设计，不使用任何第三方图标资源（包括 Typedown 的图标）。

`/register` 不抢默认应用：只填空位，绝不改写 `UserChoice`。要让 `.md` 双击默认由 Veil 打开，请在「设置 → 应用 → 默认应用」中选择 Veil（Windows 不允许程序自行设为默认）。移动 `veil.exe` 后，在旧路径 `/unregister`、新路径 `/register`；也可以手动关联：

1. 右键任意 `.md` 文件，选择 **打开方式** → **选择其他应用**。
2. 选择 **更多应用** → **在这台电脑上查找其他应用**，选中你的 `veil.exe`。
3. 勾选 **始终使用此应用打开 .md 文件**。

## 架构

四个 crate，依赖方向单向（`veil` → `veil-ui` → `veil-core`；`veil-mermaid-offline` 只依赖 `veil-core`；禁止反向）：

| Crate | 职责 |
|---|---|
| `veil-core` | 纯逻辑内核：文档模型、增量解析、编辑事务。零 GUI、零异步运行时依赖 |
| `veil-ui` | 视图适配器：winit + softbuffer 软件呈现 + cosmic-text 排版。chrome（菜单/状态栏/滚动条）自绘，不引入 egui/GL（ADR-005） |
| `veil-mermaid-offline` | Mermaid 引擎：QuickJS + 固定版本完整 `mermaid.js` + linkedom headless DOM；进程内把图表源码变成 SVG，永不联网 |
| `veil` | 产品层二进制：平台接缝（Windows FFI、配置、剪贴板、文件监听）与功能装配 |

> 2026-11-03 v1 发布前将 `veil-egui` 更名为 `veil-ui`：该 crate 自 ADR-005 起已零 egui 依赖，旧名与事实不符。crate 边界与职责未变。
>
> 只有一个发布产物 `veil.exe`。Mermaid 通过 `veil-mermaid-offline`（QuickJS + 完整 `mermaid.js`）在进程内渲染，图表源码永不离开本机。不再有第二个二进制，也没有 `veil-full.exe`：离线引擎是默认 feature，并在首次请求图表时才初始化，没有图表的文档不为它付出任何内存成本。

## 已实现功能

1. 文本编辑：插入/删除/撤销/重做（事务）
2. 增量解析与语义树（脏块局部重同步）
3. 光标与选区：grapheme 边界导航；光标进出 `**bold**` 等标记时展开/收起，光标视觉位置不跳；链接使用主题链接色（浅 `#2B6CB0` / 深 `#7FB4F5`），与正文不同色
4. 文件打开/保存：自动保存、外部修改提示与覆盖前备份
5. 剪贴板：纯文本、HTML→Markdown 转换、截图粘贴落盘（`<文档名>.assets/`，递增命名不覆盖）
6. 块级渲染：表格（网格态 + 活动单元格内直接编辑；活动格显示 Markdown 源码，其余单元格渲染粗体/斜体/删除线/行内代码/链接色并隐藏标记）、公式（latex-rust 纯 Rust 路径）、文档内图片（相对路径 PNG；断链降级为 alt 文本；远程 http/https 图片默认自动加载，取消勾选「视图 → 允许远程图片」即全局关闭；加载失败降级为 alt 占位并在状态栏提示一次）
7. HTML 导出：单文件自包含、无外部资源；内联 CSS 跟随当前浅/深主题（背景、正文、弱化色、链接色、代码底色、边框），深色页面含 `color-scheme: dark`
8. 配置与主题：顶层 `mode` 单键（system/light/dark，兼容旧 `[theme] mode`）、跟随系统外观、深色标题栏/Mica、切换持久化
9. 自绘 chrome：菜单栏（↑/↓ 选择、Enter 执行、Esc 关闭，主菜单与右键菜单一致）、状态栏、滚动条；系统文件对话框（打开/另存为）
10. 写作闭环：中文口径字数统计、专注模式（视图开关、不持久化，启动即回默认）、Ctrl+P 快速打开（最近文件 + 当前目录候选）、Ctrl+Shift+P 打印、图片粘贴/拖拽插入
11. 表格编辑：Tab / Shift+Tab 在单元格间导航（末格 Tab 追加一行并落到新行首格），Ctrl+Enter 在表尾追加空行；右键菜单支持上方/下方插入行、插入左列/右列、删除行/删除列，以及列对齐（左/居中/右，仅改写分隔行），每次结构变更一次 undo
12. 代码块语法高亮：`syntect`（fancy-regex，纯 Rust）默认语法集（75 套）；旧有 19 个规范名中 17 个被覆盖（`toml` / `dockerfile` 回退纯文本）。语法集在首次遇到代码块时惰性加载，逐行 `ParseState` 缓存使单字符编辑只重解析受影响的行。块右上角显示语言标签（已支持语言用规范名，未知语言保留 info string 原文，无语言时隐藏）与复制按钮，复制内容为代码原文（不含围栏/语言标记，不修改文档、不产生撤销、不置脏）；键盘/菜单路径：`Ctrl+Shift+C` 复制光标所在代码块，代码块右键菜单首项为「复制代码块」
13. 源码模式：`Ctrl+/` 全文显示 Markdown 源码（含表格与公式块），切换保持光标与滚动位置；打印/导出始终为渲染态
14. 导出 PDF：`Ctrl+Shift+E` 或 文件 → 导出 PDF，输出位置由你在保存对话框中确认（重名递增 ` (2)`/` (3)`）；可选目录页、页码、页边距（默认 20 mm），配置写入 `config.toml`。这是唯一的 PDF 出口：原生矢量直出（多级书签 + CJK 字体子集化 + 标题/表格/图片/公式排版），可搜索、可复制。原生导出失败时（缺少中文字体、目标目录不可写、文件被占用、成品过大）保留既有目标文件并显示可操作错误，不会打开打印对话框，也不会生成位图 PDF。要预览最终版面，请导出后用系统 PDF 查看器打开。未保存草稿会先弹保存对话框选定输出位置（取消即中止）。
15. 链接打开：`Ctrl+单击`（鼠标手势，不是新快捷键）http/https/mailto 链接时，交给系统默认浏览器/邮件客户端打开，光标不移动；链接右键菜单首项为「在浏览器中打开」，其后为「复制链接地址」；非白名单协议只提示不打开，除按需抓取文档内远程图片外，Veil 自身不发起网络请求
16. GFM 脚注：支持 `[^id]` 引用与 `[^id]: 内容` 定义。渲染态引用显示为链接色编号 `[n]`，定义块隐藏标记、显示「编号 + 内容 + ↩」；编号按全文首次引用顺序；`Ctrl+单击` 引用跳到定义、`Ctrl+单击` 定义末尾 `↩` 跳回首次引用；源码模式（`Ctrl+/`）显示原始标记；HTML 导出生成 `<sup>` 引用与文末脚注列表（含返回链接）
17. 文档内锚点跳转：`[文本](#slug)` 的 `Ctrl+单击` 跳到对应标题；导出 HTML 的标题带 `id="<slug>"`；重复标题按 `名称`、`名称-1` 去重（与导出共用同一套 `heading_slug` / `heading_slugs` 规则）
18. 行与导航编辑：`Alt+↑` / `Alt+↓` 移动当前行（一次 undo），`PgUp` / `PgDn` 翻页并保留目标列，`Ctrl+Backspace` / `Ctrl+Delete` 按词删除且不跨行
19. 查找替换：新增「区分大小写」开关（默认关、会话内保持）；正则模式（`.*` 按钮或 `Alt+R`，会话内保持）支持行锚定 `^`/`$`、捕获组替换（`$1` / `${name}`）与非法模式的就地错误提示；`Enter` 替换当前命中并前进，`Ctrl+Enter` 全部替换
20. 可读行宽：窗口宽度 >1600 px 时正文列约 40 个全角字宽并居中；H4–H6 按层级缩进；标题段距大于正文段距
21. 状态可见：状态栏显示「文件名 · 已保存 HH:MM」，脏态显示「· 未保存」；CLI 打开失败弹出可见错误框并以退出码 1 结束
22. 光标表达焦点：窗口聚焦时实心、失焦时空心，零定时器、零空闲唤醒
23. 状态过渡：菜单栏、菜单项、大纲、快速打开、右键菜单、查找栏与对话框按钮的悬停/按下高亮以 120 ms ease-out cubic 淡入淡出；仅在动画进行中推进，空闲路径仍然零定时器、不唤醒进程；系统关闭「显示动画」时直接跳到终态
24. 远程图片（v1.4）：http/https 图片默认自动加载，取消勾选「视图 → 允许远程图片」即全局关闭；抓取失败降级为 alt 占位并在状态栏提示一次。请求不携带 Cookie、`Referer` 或任何凭据，UA 固定为 `Veil/<version>`，连接/读取超时 10 s，响应体上限 20 MiB，最多跟随 3 次重定向且拒绝 https→http 降级，拒绝 localhost / 回环 / 私有 / 链路本地地址。导出与 PDF 永不包含远程资源
25. 编辑手感（v1.5）：`()` `[]` `{}` `"` `'` 五类自动配对，支持选中包裹与成对退格；光标 600 ms 闪烁在 IME 预编辑与打印时暂停；`Ctrl+,` 设置模态（9 个配置项，改动立即生效）
26. 大纲与目录：`[TOC]` 行渲染为带标题锚点的嵌套有序列表，并随编辑自动刷新
27. 图片工作流（v1.6 + 本轮）：alt 文本作为题注；独立图支持 `<img align="left|center|right">` 与显式 `width`/`height`；单击独立图片打开放大预览（滚轮平移、`Ctrl+滚轮` 或 `+`/`-` 缩放、`0` 适应、方向键/翻页平移、`Esc`/遮罩关闭；`Ctrl+单击` 仍落光标显示源码）
28. 阅读与导航：折叠的 Front Matter 卡片可点击展开；标题/TOC 锚点可跳转；可交互 chrome 带指针反馈
29. 打印与 PDF（本轮）：文件 → 打印… 打开系统打印对话框并经 GDI 出纸，复用与 PDF 导出相同的 `pdf_layout` 分页（页码范围、份数与取消由系统对话框处理）；导出 PDF 仍是唯一矢量出口。应用内不再提供打印预览——导出 PDF 后用系统查看器查看最终版面。
30. 行内 HTML 折叠：块级 `<details>` 支持点击摘要展开/收起；引用、提示与列表项内的 `<details>` 同样可折叠（仅视图态，不改写文档字节，光标进入正文自动展开）；屏幕、打印、PDF 与 HTML 导出语义一致
31. 版本历史（`Ctrl+Shift+H`）：每次覆盖保存前自动留档旧磁盘版本，脏缓冲另按 30 分钟节流自动拍快照；面板列出快照并只读预览，可恢复某一版（恢复后 `Ctrl+Z` 一步撤销）、删除单个快照或清空历史
32. 大纲面板（`Ctrl+Shift+O`）：以标题列表呈现当前文档，点击即跳转，随编辑自动刷新；无标题时给出明确空态
33. 视图控制：`Ctrl+=` / `Ctrl+-` / `Ctrl+0` 或 `Ctrl+wheel`（含触控板捏合）以 10% 步进在 50%–300% 缩放（仅会话内，不持久化）；`F11` 全屏
34. 窗口：`Ctrl+Shift+N` 新建独立窗口，当前窗口保留自己的文档、光标与脏状态
35. Mermaid 图：`mermaid` 围栏块始终保留源码卡与模板。可见图块在进程内本地渲染（QuickJS + 完整 `mermaid.js`，含 mindmap 在内的全图型），编辑源码后防抖重渲；引擎在首次请求图表时才创建，没有图表的文档不为它付出成本。渲染永不联网——源码留在本机，失败时给出本地可操作错误，不回退到远程服务。源码始终是唯一真值：光标进入时显示带代码底色、等宽字体与复制按钮的围栏。图块右键菜单提供 **复制源码**、**在 mermaid.live 打开**（复制源码并打开站点；URL 不携带源码，Veil 自身不上传）与 **插入图片快照**——把图写入 `<doc>.assets/mermaid-N.png` 并在块后追加 `![](相对路径)`，一条 undo 回退。旧配置 `mermaid_insert_url` 已静默忽略：只写本地快照，该动作绝不会插入外部图片 URL。快照是独立、可能过期的图片，不是可双向编辑的活图，源码不会被删除或永久隐藏

## 非目标（仍未纳入）

- wgpu/GPU 渲染视图（裁决见 ADR-011：v1 不做，仅在触发条件满足时立项）
- 协作编辑 / CRDT（**永久排除 — Veil 单机单用户定位**）
- 拼写检查
- 插件系统
- 移动端 / PWA（仅桌面 exe）
- 表格结构可视化编辑的剩余部分（合并/拆分单元格）；内容编辑、行列增删、列对齐与列宽拖动已支持
- 用于图表渲染的浏览器引擎或 WebView：Mermaid 由 `veil-mermaid-offline`（QuickJS + 完整 `mermaid.js`）在进程内渲染，不含 Chromium；输出统一禁用 `foreignObject`，使 resvg 能栅格化标签
- 独立富文本文档模型：正文始终是 Markdown，`Ctrl+/` 只是同一文档的视图切换
- 任何后台网络：无同步、无更新检查、无遥测，Mermaid 永不联网。普通远程图片仍是独立的显式能力（「视图 → 允许远程图片」），关闭它不影响 Mermaid。外部链接与 `mermaid.live` 只在用户显式操作时打开。导出 / PDF 永不包含远程资源。
- 竞品对齐项明确不做：独立的全词匹配开关（正则模式的 `\b` 已覆盖）、`Ctrl+Shift+K` 删除行（`Alt+↑` / `Alt+↓` 已覆盖行级操作）、独立 H5/H6 字阶（层级用缩进表达）。括号/引号自动配对已于 v1.5 交付，不再属于非目标

## 已知限制（渲染与导出）

Veil 不宣称所有 Markdown / Mermaid 语法在编辑态、源码态、HTML、PDF、打印五个面都 100% 一致。以下是已知限制，不是「即将支持」的承诺：

- **Mermaid 可读性**：`gantt` 的任务与分节文字通过上游公开的 `gantt.fontSize` / `sectionFontSize` 从默认 11 px 提升到 14 px，由进程内渲染器设置。日期刻度字号与内部布局度量仍由 Mermaid 决定。**局限**：`c4context` 的 stereotype 文字由上游 c4 渲染器固定为约 12 px，没有安全的放大参数（放大 CSS 会超出按 12 px 量出的固定方框），因此记录为**已知降级**，不做假装修复。
- **深色 Mermaid 对比度**：自动断言覆盖 `mindmap`、`kanban`、`journey`、`gantt`、`c4context` 的文字/底色对比度（目标 WCAG AA）。其余图型只校验几何、可见文字与出图成功；深色主题的逐图人工目检仍为 **NOT RUN**。`c4context` 的连线标签沿用 Mermaid 固定的 `#444444`，深色背景下可能对比不足；用全局 CSS 覆盖会泄漏到其他图型，因此该项未加断言，记为 **NOT RUN**。
- **PDF / 打印表格**：单元格文字现在保留粗体、斜体、行内代码底色、链接颜色/下划线及链接注解；超长单元格改为跨页续排，不再截断。分页按固定行高估算拆行，极端高的单元格可能比浏览器多占几页。
- **PDF / 打印中文断行**：已加入简化禁则（闭符号不出现在行首、开符号不留在行尾），但不是完整的 UAX #14 断行与标点挤压，换行位置仍可能与编辑态（cosmic-text）不同。
- **远程图片**：编辑态可按需显示，但 HTML 导出会删除远程 `src`（保证自包含），PDF/打印降级为 alt 文本或 `[图片]`；导出与 PDF 永不下载远程资源。
- **图片采样**：Mermaid 位图先还原 CSS 尺寸并限制在版心内再绘制，但使用最近邻缩放；超大图受版心宽度限制，不会按缩放级别重新光栅化。
- **Details**：编辑态默认折叠；HTML、PDF 与打印恒展开。这是有意行为，不是跨面缺陷。

## 构建 / 运行 / 测试

前置：Rust stable（edition 2021）。验收环境：Windows 11 23H2+；Mica 亚克力在不可用时自动降级为纯色；Windows 10 未做真机记录，不宣称已验证。

界面文案仅简体中文（zh-CN）：i18n 是单语词条表，没有英文条目，README 中引用的英文标签是对实际中文文案的翻译。Veil 不探测系统显示语言，也没有运行时语言切换。正文、导出内容与图内文字不随界面翻译。

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>

# 发布产物：只有一个 veil.exe，默认包含离线 Mermaid
cargo build --release --bin veil
cargo test -p veil-mermaid-offline
```

当前 1911 项测试全绿（串行 `cargo test --workspace --release -- --test-threads=1`：1911 passed / 0 failed / 15 ignored，2026-11-20；远程 SVG mock 使用 20 ms 墙钟截止，并行执行可能假超时）；默认特性下 `cargo test -p veil --lib` 为 221 passed / 1 ignored（默认特性现已包含离线引擎）。`cargo clippy --workspace --all-targets -- -D warnings` 零警告。

## 零级门禁：真实启动冒烟

测试全绿不等于用户双击的 exe 能打开。发布前必须对 release 产物运行真实启动冒烟：

```powershell
pwsh -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000
```

出窗预算是 **1 秒**：`-MaxLaunchMs`（默认 1000）测量「进程启动 → `MainWindowHandle` 非零」，超限即 `stage=launch-too-slow` 判 FAIL——即使窗口最终出现、标题正确、退出码为 0 也不例外。`-TimeoutSeconds`（默认 5 秒）只决定探测时长，用于区分「慢启动」与「压根没出窗」，绝不放宽 1 秒预算。脚本还会校验标题非空（给出 `-ExpectedTitlePattern` 时还需匹配），即使窗口已出现，只要 stderr 含 `panicked at` / `buffer_mut` / `Must set size of surface` 也判失败；随后请求正常关闭并检查退出码。失败时非零退出，并打印结构化 `SMOKE FAIL ... stage=process-exited|window-timeout|launch-too-slow|stderr-panic|...` 行，PASS 行同时包含 `launch_ms` 与 `max_ms`。启动前若已有同名 `veil` 进程，脚本会明确失败并要求先手动关闭，绝不杀它未启动的进程。

**冒烟失败一律不得发布**，单元测试、clippy、体积门禁、截图都不能替代，且发布前必须在本地对最终产物运行。发布阻断优先级：冒烟第一；CI 因必须验证最终要上传的产物，在测试/边界门禁之后、上传之前执行同一脚本，且不得使用 `continue-on-error`。GitHub 托管 Windows runner 可能没有交互桌面（拿不到 `MainWindowHandle`），因此权威口径是本地运行；若本地 PASS 而 CI 出现纯环境性失败，应在 release notes 记录并附本地日志，而不是削弱门禁。

## 数字红线（目标值与实测）

发布构建、探针串行；性能项 2026-11-18 复核，测试与体积项 2026-11-18 复核。命令：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo run --release -p veil --example bench_typing`、`cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path`（10,500 块用 `perf_keystroke_10k_blocks`）、`cargo build --release -p veil` + `(Get-Item target\release\veil.exe).Length`，依赖计数用 `cargo tree -p veil --edges normal --prefix none`。方法与明细见 `docs/review/keystroke-attribution.md`。

| 指标 | 目标值 | 实测 |
|---|---|---|
| 冷启动（发布构建，`main`→首帧完成；**验收口径**；5 次） | ≤ 300 ms | 145–190 ms（最小–最大；2026-11-05 `VEIL_BOOT_LOG` 分段探针）✅ |
| 冷启动（进程内打开 + 首次全量排版 + 首次编辑；5 次 p50 / max） | 观察项 | 405.7 ms（应用 188.9 ms + 首编辑 216.8 ms；2026-11-18，`cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`）。仅代表本机：受系统文件缓存影响，同机重复运行在 290–580 ms 间波动 |
| 冷启动（bench 工具，进程创建→打开→首屏排版；p50 / max） | 观察项 | 576.46 / 599.98 ms（2026-11-09，`cargo run --release -p veil --example bench_typing`；基线 520.87 / 533.85 → +10.7% / +12.4%，回退门禁 ≤ 20%） |
| 冷启动（墙钟，进程创建→首帧；环境观察项） | ≤ 300 ms | 450–529 ms（其中 440–470 ms 在 `main` 之前，属本机进程创建段开销；2026-10-31） |
| 冷启动（窗口句柄口径；P2 对比口径） | — | 30 ms（2026-10-30，`tools/bench/measure.ps1`） |
| 10 万字文档击键→提交上屏延迟 p95 | ≤ 16 ms | 0.973 ms / 1.566 ms（`perf_keystroke_path` 5,005 块 / `perf_keystroke_10k_blocks` 10,500 块；同机 2026-11-18 复测；上一轮记录 1.297 / 1.796）✅ |
| 10 万字文档常驻内存（私有工作集） | ≤ 150 MB | 40.76 MB（20 页深滚动后 WorkingSet，2026-11-05 实测） |
| 安装产物体积 — 合计单文件 `veil.exe` | ≤ 14 MiB = 14,680,064 B | 13.35 MiB = 13,995,008 B（2026-11-20，单一产物：离线 Mermaid 引擎 + 完整 `mermaid.js` + resvg，引擎惰性初始化） |
| `veil-core` 单元+属性测试数 | ≥ 80 | 347 |
| e2e 断言数 | ≥ 20 | 41（3 个 e2e_ 场景测试：写作闭环打开→保存→重开、表格+图片粘贴落盘、查找替换→崩溃恢复→自清）✅ |
| Markdown 往返一致率（无编辑时输出 == 输入字节；按字节比较） | 100% | 100% |
| CommonMark 0.31.2 一致性（按用例数；A / C / D 维） | — | A 652/652 · C 652/652 · D 652/652 ✅ |
| CommonMark 0.31.2 一致性（B 维） | — | B_EXPORT_EXACT 629/652 · B_CANON 630/652（非全通过） |
| GFM 扩展一致性（Tables / TaskList / Strikethrough；按用例数；A / C / D 维） | — | A 23/23 · C 23/23 · D 23/23 ✅；B_CANON 12/23（11 例 autolinks 仅观测） |
| `veil-core` 的 GUI/异步运行时依赖数 | 0 | 0 |
| `veil-ui`+`veil` 外部直接依赖总数（`cargo tree --depth 1`，按 crate 名去重） | ≤ 8（PDF 前置基线；已被 Wave 3 授权引入的 `pdf-writer` + `subsetter` + `ttf-parser` 与 v1.8 的 `regex` 覆盖） | 17（`veil-ui` 8 + `veil` 9） |
| workspace 传递依赖总数（按 crate 名去重，normal 边，含三个 workspace 成员） | ≤ 200 | 125（2026-11-18，`cargo tree -p veil --edges normal --prefix none`） |

## 同机对比（P2 公开基准，2026-10-30）

语料与脚本可复现：用 `tools/bench/gen_corpus.py` 生成语料、`tools/bench/measure.ps1` 测量；语料与结果不入库（10 MB 病态语料按需生成）。方法与口径声明见 [P2-benchmark.md](docs/review/P2-benchmark.md)。

| 指标 | Veil | MarkText | 倍率 |
|---|---|---|---|
| 常驻内存（进程树求和） | 21.51 MB | 365.01 MB | **17.0×** |
| 安装产物体积 | 5.46 MB | 488.95 MB | **89.6×** |
| 冷启动（窗口句柄口径） | 30 ms | 30 ms | 打平（该口径系统性偏袒 Electron，见报告 §3） |

依赖黑名单：`tokio`、`wasmtime`、一切 webview crate（`webview2-com`/`wry` 等），引入即视为违规。

## 文档导航

- `docs/review/v1-2-implementation-log.md` — v1.2 实施记录：实测数字、探针、不做项裁定
- `docs/review/keystroke-attribution.md` — 击键到提交的延迟归因
- `docs/adr/` — 架构决策记录：
  - ADR-001 rope 选型
  - ADR-002 公式渲染路径
  - ADR-003 光标与编辑点规则
  - ADR-004 平台接缝
  - ADR-005 chrome 自绘 + 系统对话框
  - ADR-006 外部文件监听
  - ADR-007 配置与主题
  - ADR-008 依赖升级策略
  - ADR-009 状态模型（单一真值、正交状态机、因果派生）
  - ADR-010 协作位置模型（v2 设计、v1 冻结 —— Veil 始终本地单机）
  - ADR-011 wgpu 延迟引入（v1 不引入，触发式立项）
  - ADR-012 语法高亮从自研状态机迁移到 `syntect`（fancy-regex），按行增量缓存
  - ADR-013 表格布局保持自研（不引入 `comfy-table`）
  - ADR-014 Quick Open 模糊匹配保持手写（不引入 `fuzzy-matcher`/`nucleo`）
  - ADR-012 代码块高亮迁移到 syntect（fancy-regex + 逐行增量缓存）
- `.trae/specs/release-and-roadmap/v1.1-roadmap.md` — v1.1 详细 roadmap
- `.trae/specs/release-and-roadmap/v1.2-roadmap.md` — v1.2 候选池（排版设置、PDF 书签、字体子集化）
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 详细 roadmap（wgpu 后移四条触发条件同时满足才启动）

## 许可

MIT License · see [LICENSE](LICENSE)。
