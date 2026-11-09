# Veil

![Veil](icons/veil.ico)

[![CI](https://github.com/31i45/veil/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/31i45/veil/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/31i45/veil?label=release)](https://github.com/31i45/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable%20%C2%B7%20edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%2011%20%C2%B7%20x64-0078d4.svg)](https://github.com/31i45/veil/releases)
[![Portable](https://img.shields.io/badge/single%20exe%20%C2%B7%20no%20installer%20%C2%B7%20offline-2ea44f.svg)](https://github.com/31i45/veil/releases)

[English](README.md)

## Veil 的起源

Veil 的灵感来自 Typedown — Typedown 是我最喜欢的 Windows Markdown 编辑器，没有之一，但是它启动需要几十秒到数分钟，每次都让人崩溃。Typedown 是 WinUI 框架实现的应用，冷启动快过你完成一杯咖啡的时间本该是理所当然的事，但现实并非如此。

所以，我决定自己复刻。纯 Rust 实现，无 webview、无 Chromium/WebView2 进程、无外部运行时依赖。单文件 exe，应用内冷启动 ≈150 ms（首次打开并编辑约再增 160 ms；双指标与测量命令/日期见[数字红线](#数字红线目标值与实测)）。**单机单用户 · 纯离线 · 永不协作**。

## 下载与使用

从 [Releases](https://github.com/31i45/veil/releases) 下载 `veil.exe`。无需安装、无外部运行时依赖、不需要管理员权限。

1. 把 `veil.exe` 放到一个打算长期保留的目录（例如 `%LOCALAPPDATA%\Veil`）。Windows 按路径记录文件关联，之后移动 exe 会让关联失效。
2. 双击运行，或执行 `veil.exe 某个文件.md`。

## 把 .md 文件关联到 Veil

Veil 是便携单文件程序，不写注册表，因此无法自动把自己注册为 `.md` 的默认打开方式。手动关联步骤：

1. 右键任意 `.md` 文件，选择 **打开方式** → **选择其他应用**。
2. 选择 **更多应用** → **在这台电脑上查找其他应用**，选中你的 `veil.exe`。
3. 勾选 **始终使用此应用打开 .md 文件**。

如果之后移动了 `veil.exe`，请在新位置重复上述步骤。

## 架构

三个 crate，依赖方向单向（`veil` → `veil-ui` → `veil-core`，禁止反向）：

| Crate | 职责 |
|---|---|
| `veil-core` | 纯逻辑内核：文档模型、增量解析、编辑事务。零 GUI、零异步运行时依赖 |
| `veil-ui` | 视图适配器：winit + softbuffer 软件呈现 + cosmic-text 排版。chrome（菜单/状态栏/滚动条）自绘，不引入 egui/GL（ADR-005） |
| `veil` | 产品层二进制：平台接缝（Windows FFI、配置、剪贴板、文件监听）与功能装配 |

> 2026-11-03 v1 发布前将 `veil-egui` 更名为 `veil-ui`：该 crate 自 ADR-005 起已零 egui 依赖，旧名与事实不符。crate 边界与职责未变。

## 已实现功能

1. 文本编辑：插入/删除/撤销/重做（事务）
2. 增量解析与语义树（脏块局部重同步）
3. 光标与选区：grapheme 边界导航；光标进出 `**bold**` 等标记时展开/收起，光标视觉位置不跳；链接使用主题链接色（浅 `#2B6CB0` / 深 `#7FB4F5`），与正文不同色
4. 文件打开/保存：自动保存、外部修改提示与覆盖前备份
5. 剪贴板：纯文本、HTML→Markdown 转换、截图粘贴落盘（`<文档名>.assets/`，递增命名不覆盖）
6. 块级渲染：表格（网格态 + 活动单元格内直接编辑；活动格显示 Markdown 源码，其余单元格渲染粗体/斜体/删除线/行内代码/链接色并隐藏标记）、公式（latex-rust 纯 Rust 路径）、文档内图片（相对路径 PNG，断链/远程 URL 降级为 alt 文本）
7. HTML 导出：单文件自包含、无外部资源；内联 CSS 跟随当前浅/深主题（背景、正文、弱化色、链接色、代码底色、边框），深色页面含 `color-scheme: dark`
8. 配置与主题：顶层 `mode` 单键（system/light/dark，兼容旧 `[theme] mode`）、跟随系统外观、深色标题栏/Mica、切换持久化
9. 自绘 chrome：菜单栏（↑/↓ 选择、Enter 执行、Esc 关闭，主菜单与右键菜单一致）、状态栏、滚动条；系统文件对话框（打开/另存为）
10. 写作闭环：中文口径字数统计、专注模式与打字机模式（两个视图开关彼此互斥、全关即默认模式、不持久化，启动即回默认）、Ctrl+P 快速打开（最近文件 + 当前目录候选）、Ctrl+Shift+P 打印、图片粘贴/拖拽插入
11. 表格编辑：Tab / Shift+Tab 在单元格间导航（末格循环回首格），Ctrl+Enter 在表尾追加空行；右键菜单支持上方/下方插入行、插入左列/右列、删除行/删除列，以及列对齐（左/居中/右，仅改写分隔行），每次结构变更一次 undo
12. 代码块语法高亮：16 种语言的关键字/字符串/注释/数字/类型着色（纯 Rust 逐行状态机，不引入 syntect）
13. 源码模式：`Ctrl+/` 全文显示 Markdown 源码（含表格与公式块），切换保持光标与滚动位置；打印/导出始终为渲染态
14. 导出 PDF：`Ctrl+Shift+E` 或 文件 → 导出 PDF，默认输出到文档同目录 `<stem>.pdf`（重名递增 ` (2)`/` (3)`）；可选目录页、页码、页边距（默认 20 mm），配置写入 `config.toml`；依赖系统「Microsoft Print to PDF」。未保存草稿会先弹保存对话框选定输出位置（取消即中止），无 PDF 打印机时回退系统打印对话框并提示。已知限制：无 PDF 书签、无字体子集化
15. 链接打开：`Ctrl+单击`（鼠标手势，不是新快捷键）http/https/mailto 链接时，交给系统默认浏览器/邮件客户端打开，光标不移动；链接右键菜单首项为「在浏览器中打开」，其后为「复制链接地址」；非白名单协议只提示不打开，Veil 自身不发起网络请求
16. GFM 脚注：支持 `[^id]` 引用与 `[^id]: 内容` 定义。渲染态引用显示为链接色编号 `[n]`，定义块隐藏标记、显示「编号 + 内容 + ↩」；编号按全文首次引用顺序；`Ctrl+单击` 引用跳到定义、`Ctrl+单击` 定义末尾 `↩` 跳回首次引用；源码模式（`Ctrl+/`）显示原始标记；HTML 导出生成 `<sup>` 引用与文末脚注列表（含返回链接）
17. 文档内锚点跳转：`[文本](#slug)` 的 `Ctrl+单击` 跳到对应标题；导出 HTML 的标题带 `id="<slug>"`；重复标题按 `名称`、`名称-1` 去重（与导出共用同一套 `heading_slug` / `heading_slugs` 规则）
18. 行与导航编辑：`Alt+↑` / `Alt+↓` 移动当前行（一次 undo），`PgUp` / `PgDn` 翻页并保留目标列，`Ctrl+Backspace` / `Ctrl+Delete` 按词删除且不跨行
19. 查找替换：新增「区分大小写」开关（默认关、会话内保持）；`Enter` 替换当前命中并前进，`Ctrl+Enter` 全部替换
20. 可读行宽：窗口宽度 >1600 px 时正文列约 40 个全角字宽并居中；H4–H6 按层级缩进；标题段距大于正文段距
21. 状态可见：状态栏显示「已保存 · HH:MM」（或「未保存」）；CLI 打开失败弹出可见错误框并以退出码 1 结束
22. 光标表达焦点：窗口聚焦时实心、失焦时空心，零定时器、零空闲唤醒

## 非目标（v1.2 明确不做）

- wgpu/GPU 渲染视图（裁决见 ADR-011：v1 不做，仅在触发条件满足时立项）
- 协作编辑 / CRDT（**永久排除 — Veil 单机单用户定位**）
- 拼写检查
- 插件系统
- 移动端 / PWA（仅桌面 exe）
- 表格结构可视化编辑的剩余部分（合并/拆分单元格、列宽拖动）；内容编辑、行列增删与列对齐已支持
- Mermaid 渲染（Mermaid 围栏块保持为代码，不做渲染）
- 独立富文本文档模型：正文始终是 Markdown，`Ctrl+/` 只是同一文档的视图切换
- 任何网络功能：无同步、无更新检查、无遥测；外部链接交给系统默认程序打开，Veil 自身不发起网络请求
- 竞品对齐项明确不做：正则查找（`regex` 实测使 exe 净增 1,198,592 B）、括号/引号自动配对、全词匹配开关、`Ctrl+Shift+K` 删除行（`Alt+↑` / `Alt+↓` 已覆盖行级操作）、≤120 ms 状态过渡动画（与零定时器/无空闲唤醒策略冲突）、独立 H5/H6 字阶（层级用缩进表达）

## 构建 / 运行 / 测试

前置：Rust stable（edition 2021）。验收环境：Windows 11 23H2+；Mica 亚克力在不可用时自动降级为纯色；Windows 10 未做真机记录，不宣称已验证。

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>
```

当前 1132 项测试全绿（`cargo test --workspace` 1132 passed / 0 failed / 13 ignored，2026-11-09 实测），`cargo clippy --workspace --all-targets -- -D warnings` 零警告。

## 数字红线（目标值与实测）

发布构建、探针串行，2026-11-09 复核。命令：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo run --release -p veil --example bench_typing`、`cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path`（10,500 块用 `perf_keystroke_10k_blocks`）、`(Get-Item target\release\veil.exe).Length`。方法与明细见 `docs/review/keystroke-attribution.md`。

| 指标 | 目标值 | 实测 |
|---|---|---|
| 冷启动（发布构建，`main`→首帧完成；**验收口径**；5 次） | ≤ 300 ms | 145–190 ms（最小–最大；2026-11-05 `VEIL_BOOT_LOG` 分段探针）✅ |
| 冷启动（进程内打开 + 首次全量排版 + 首次编辑；5 次 p50 / max） | 观察项 | 290.0 ms（应用 131.8 ms + 首编辑 158.2 ms；预热后 130.1 ms；2026-11-09，`cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`）。仅代表本机：受系统文件缓存影响，同机重复运行在 290–580 ms 间波动 |
| 冷启动（bench 工具，进程创建→打开→首屏排版；p50 / max） | 观察项 | 576.46 / 599.98 ms（2026-11-09，`cargo run --release -p veil --example bench_typing`；基线 520.87 / 533.85 → +10.7% / +12.4%，回退门禁 ≤ 20%） |
| 冷启动（墙钟，进程创建→首帧；环境观察项） | ≤ 300 ms | 450–529 ms（其中 440–470 ms 在 `main` 之前，属本机进程创建段开销；2026-10-31） |
| 冷启动（窗口句柄口径；P2 对比口径） | — | 30 ms（2026-10-30，`tools/bench/measure.ps1`） |
| 10 万字文档击键→提交上屏延迟 p95 | ≤ 16 ms | 6.781 ms / 12.379 ms（`perf_keystroke_path` 5,005 块 / `perf_keystroke_10k_blocks` 10,500 块；基线 6.677 / 12.488；2026-11-09）✅ |
| 10 万字文档常驻内存（私有工作集） | ≤ 150 MB | 40.76 MB（20 页深滚动后 WorkingSet，2026-11-05 实测） |
| 安装产物体积（`veil.exe` 单文件；无安装目录/解压后额外总量） | ≤ 6 MiB = 6,291,456 B | 6,043,136 B ≈ 5.76 MiB（2026-11-09 实测）；余量 248,320 B |
| `veil-core` 单元+属性测试数 | ≥ 80 | 209 |
| e2e 断言数 | ≥ 20 | 41（3 个 e2e_ 场景测试：写作闭环打开→保存→重开、表格+图片粘贴落盘、查找替换→崩溃恢复→自清）✅ |
| Markdown 往返一致率（无编辑时输出 == 输入字节；按字节比较） | 100% | 100% |
| CommonMark 0.31.2 一致性（按用例数；A / C / D 维） | — | A 652/652 · C 652/652 · D 652/652 ✅ |
| CommonMark 0.31.2 一致性（B 维） | — | B_EXPORT_EXACT 629/652 · B_CANON 630/652（非全通过） |
| GFM 扩展一致性（Tables / TaskList / Strikethrough；按用例数；A / C / D 维） | — | A 23/23 · C 23/23 · D 23/23 ✅；B_CANON 12/23（11 例 autolinks 仅观测） |
| `veil-core` 的 GUI/异步运行时依赖数 | 0 | 0 |
| `veil-ui`+`veil` 直接依赖总数 | ≤ 8 | 8 |
| workspace 传递依赖总数 | ≤ 200 | 141 |

## 同机对比（P2 公开基准，2026-10-30）

语料与脚本可复现：`bench/corpus/`（生成器 `tools/bench/gen_corpus.py`，其中 10MB 病态语料不入库、按需生成）、`tools/bench/measure.ps1`。方法与口径声明见 [P2-benchmark.md](docs/review/P2-benchmark.md)。

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
  - ADR-011 wgpu 延迟引入（v1 不引入，触发式立项）
- `.trae/specs/release-and-roadmap/v1.1-roadmap.md` — v1.1 详细 roadmap
- `.trae/specs/release-and-roadmap/v1.2-roadmap.md` — v1.2 候选池（排版设置、PDF 书签、字体子集化）
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 详细 roadmap（wgpu 后移四条触发条件同时满足才启动）

## 许可

MIT License · see [LICENSE](LICENSE)。
