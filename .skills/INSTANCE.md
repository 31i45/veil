# INSTANCE.md · 项目实例文件（Veil）

> 本文件是 ai-dev-skills 通用方法论在本项目的全部实例化：项目专有事实（命令、红线、工具名、平台名、专有清单）只存在于此，skills 本体不内嵌。
> 有效期：下列任一事实变更时需更新
> 最后验证：2026-10-26（架构重构 S0~S5 落地：veil-core syntax/ 四分；veil-egui 三域分离——document/{doc_state,block_index}、view/{cache,layout,frame,style,table}、chrome/{mod,paint,cache}，app.rs/interaction.rs/paint.rs 归位根级，shell/ 消解；workspace 447 测试全绿、clippy 零警告、红线复测见 §2）

## 0. 项目骨架事实

- Workspace 三 crate，命名定型：`veil-core`（纯逻辑内核，无 GUI/异步依赖）、`veil-egui`（winit + softbuffer + cosmic-text 视图适配器，自绘 chrome，无 GL/egui）、`veil`（产品层二进制）。
- 依赖方向单向：`veil` → `veil-egui` → `veil-core`，禁止反向。
- 目录：`crates/veil-core`、`crates/veil-egui`、`crates/veil`。
- v1 目标平台：Windows 桌面。macOS/Linux 不承诺，验证时按 Skill 03 A1 标「半」。
- 已钉死的决策（详见 §7 ADR 清单）：
  - Markdown 源码是唯一真值；Reveal 等注解不持久化。
  - 内核单线程主编辑模型；后台任务（公式渲染）经消息队列回主线程，禁止跨线程改内核。
  - 内核零 GUI、零异步运行时依赖。
  - 非目标：wgpu 视图、协作/CRDT、插件系统、移动端、表格可视化编辑（v1 仅渲染+源码编辑）、Mermaid 内嵌渲染。
- crate 名 `veil-egui` 为历史遗留（ADR-005 后已无 egui 依赖），v1 不更名，v2 再评估。

## 1. 真机验证实例（Skill 03 A1）

- 命令：`cargo test --workspace`（单元/集成）；Windows 端到端 `pwsh .\e2e.ps1`（⏳ 待建）
- 场景：

| 场景 | Windows 11（基线） | macOS / Linux |
|---|---|---|
| 工具启动（冷启动 ≤300ms） | ⏳ | 半 |
| 核心功能执行（典型输入：中文 IME 写作 + 嵌套 inline） | ⏳ | 半 |
| 核心功能执行（含自定义资源：相对路径图片、公式） | ⏳ | 半 |
| 产物运行（导出 HTML 在浏览器打开渲染正确） | ⏳ | 半 |
| 退出零残留（无临时文件、配置正确落盘） | ⏳ | 半 |
| 重复执行（幂等：连续打开保存同一文件字节不变） | ⏳ | 半 |
| 无效输入不可误运行（损坏 UTF-8、0 字节文件、10MB 单文件不崩溃） | ⏳ | 半 |

- 抽检：① 微软拼音连续输入 200 字，候选框跟随光标不漂移；② 光标进出 `**bold**` 标记展开/收起，光标视觉位置不跳；③ 10 万字文档滚动到底部编辑，无全文档卡顿；④ 列表符号 `•`/引用左竖条/分隔线随光标进出切换显隐，行高不跳变；⑤ 相对路径 PNG 在行内/列表内渲染且超版心宽时等比缩放，点击图片进入源码态，断链/远程 URL 降级 alt 文本不崩溃；⑥ 截图后 Ctrl+V：`<文档名>.assets/` 生成且插入 `![](相对路径)`，重复粘贴递增 `image-2.png` 不覆盖，草稿态拒绝并保持文档不变；⑦ 菜单栏「文件/编辑」点击展开与切换、菜单项快捷键提示与悬停高亮、Ctrl+O 系统打开对话框、Ctrl+Shift+S 另存为（成功后标题去脏标/基线切换、取消无副作用）、长文档滚动条拖动与轨道翻页、状态栏行列随光标实时更新、菜单打开时 Esc 关闭且不误编辑；⑧ 外部编辑器改写当前文件：标题/状态栏出现「外部修改」、脏文档自动保存挂起、Ctrl+S 先备份（`.backup-*`）再覆盖；外部删除文件：提示「文件缺失」、继续编辑后自动保存重建文件；⑨ 「视图」菜单切换跟随系统/浅色/深色即时生效且「当前」标记移动、重启后保持、`%APPDATA%\Veil\config.toml` 内容与选择一致、系统深色模式下标题栏变深、Mica 在 Win11 显示于边框区域。
- 最近全绿：2026-10-26 架构重构 S0~S5：workspace 447 测试全绿（veil-core 90 单测 + 61 集成、veil-egui 274 + 2 ignored、veil 22）、clippy 零警告；三域分离目录——`document/`（doc_state 文档持久化编排 + block_index 块几何索引）、`view/`（cache 派生缓存统一收口与失效入口方法族 + layout/frame/style/table）、`chrome/`（mod 几何与菜单模型 / paint 绘制 / cache 标签缓存）、根级 `app.rs`（App 结构体 + ApplicationHandler + 平台 trait）、`interaction.rs`（交互策略 + 几何编排）、`paint.rs`（帧合成 + ensure_* 缓存补齐）、`app_tests.rs`（跨域行为测试）；veil-core `syntax/{mod,model,block,inline}`；每切片独立可 revert commit（5592b19→7662214）；历史状态：阶段二 P1a~P6（444 测试：块几何索引击键 p95 13.25→4.07 ms、视口惰性排版、Frame 单缓冲帧合成 4.22→0.28 ms、shell.rs 3916 行首拆、配置单键、撤销分组、删除注解层）、切片 17 配置与主题（430）、16 文件系统监听（407）、15 自绘 chrome + Win32 对话框（395）、14 图片粘贴落盘、13 文档内图片、10~12 自动保存/冲突备份/富文本粘贴/列表引用分隔线；场景表逐格仍待真机人工验证，上列 ①~⑨ 尤需人工

## 2. 数字红线（Skill 03 A2）

| 数字 | 红线/目标 | 实测 |
|---|---|---|
| 冷启动（发布构建，进程创建→首帧可交互，本机） | ≤ 300 ms | **超标**：进程创建→主窗口句柄 5 次中位 771 ms（最小 414 ms；进程内初始化实测 ~150 ms，其余为进程加载与 Defender 实时扫描）。2026-10-25 决策：如实记录，标注为 Windows 平台成本，不设缓解项 |
| 10 万字文档击键→提交上屏延迟 p95（`cargo test -p veil-egui --release -- --ignored --nocapture perf_keystroke_path`） | ≤ 16 ms | 4.07 ms ✓（5005 块 ≈16 万字，apply+after_edit+paint 端到端 300 次；p50 3.27 ms；P1a 前 13.25 ms） |
| 10 万字文档常驻内存（任务管理器私有工作集） | ≤ 150 MB | 17.3 MB ✓（100,480 字符文档，启动后 2 s 采样，5 次一致） |
| 安装产物体积（`veil.exe` 单文件发布构建） | ≤ 15 MiB | 5,489,152 B ≈ 5.23 MiB ✓（2026-10-26 S5 后实测；含 latex-rust 内嵌 STIX 字体 838,652 B） |
| `veil-core` 单元+属性测试数 | ≥ 80 | 151（2026-10-26：90 单测 + 61 集成；S1 syntax 四分纯搬移零增减） |
| e2e 断言数 | ≥ 20 | 0 |
| Markdown 往返一致率（无编辑时 serialize 输出 == 输入字节） | == 100% | 100%（`roundtrip_identity_without_edits` + `prop_random_edits_roundtrip_and_undo` 随机语料属性测试） |
| `veil-core` 的 GUI/异步运行时依赖数（`cargo tree -p veil-core` grep egui/winit/tokio） | == 0 | 0 ✓（2026-10-24 实测） |
| `veil-egui`+`veil` 直接依赖总数（`cargo tree -p veil-egui -p veil --depth 1` 去重） | ≤ 8 | 8 ✓（winit/softbuffer/cosmic-text/arboard/pulldown-cmark/latex-rust/png/notify；切片 16 新增 notify 即达预算上限，后续不得再加直接依赖） |
| workspace 传递依赖总数（`cargo tree --prefix none --depth 100 | sort -u` 去重） | ≤ 200 | 121 ✓（2026-10-26 S6 复测；重构零依赖变更。2026-10-25 升级后口径：cosmic-text 0.19 + latex-rust 3.0.1；ttf-parser 与 rustybuzz 已消除，新增 skrifa/read-fonts/font-types 各三版本，全部上游锁定） |

- 版本号一致性：git tag == workspace `Cargo.toml` version == 产物元数据（`veil.exe` 文件属性版本）
- 版本失配处置记录：无
- 测量方法注记：冷启动 = PowerShell `Start-Process` 后按进程 `StartTime` 计到主窗口句柄就绪，5 次取中位（含启动器与 Defender 扫描开销，方差大，故同时记录最小值）；击键 p95 = 进程内端到端探针 `perf_keystroke_path`（release 构建，300 次样本）；内存 = 启动后 2 s 的 `PrivateMemorySize64`；体积 = `target/release/veil.exe` 字节数。

## 3. 核心不变量断言（Skill 03 A4）

- 派生不变量：`veil-egui`/`veil` 的一切文本变更必须经 `veil-core` 单一入口（`Session::apply`）；断言方式：代码评审 + `cargo tree -p veil-core` 依赖红线（§2）CI 门禁。
- 幂等不变量：打开 → 不做任何编辑 → 保存，输出文件与输入字节级一致；重复执行不膨胀。断言方式：属性测试（任意合法 GFM 语料 round-trip）+ e2e「重复执行」场景。
- 局部性不变量：文档中部单字符插入后，内核报告的 dirty block 集合 ⊆ 重同步规则（向前扩展到所在块起点、向后扫至块边界重新同步）允许的最大范围。断言方式：固定语料 + 固定编辑位点的 dirty-block 断言测试。
- Mark 稳定不变量：任意编辑后，编辑点之外的既有 mark 所指向的文本内容不变。断言方式：编辑前后对比 mark 处子串的属性测试。

## 4. 核心功能清单（Skill 02 B3）

1. 文本编辑：插入/删除/撤销/重做（事务）
2. 增量解析与语义树（含脏块重同步规则）
3. Reveal 消隐：光标处展开 Markdown 标记（视图层 OffsetMap 换算，内核对源码零改写；内核对 Reveal 不存任何状态）
4. 光标与选区（grapheme 边界、inline 边界 mark bias 规则）
5. 文件打开/保存（含自动保存与备份）
6. 剪贴板：纯文本 / HTML→Markdown 转换 / 图片粘贴落盘
7. 块级渲染调度：表格（只读渲染）、公式（latex-rust 纯 Rust 路径，见 ADR-002）、文档内图片（相对路径 PNG，v0 PNG-only；非 PNG/断链/远程 URL 降级 alt 文本）
8. HTML 导出
9. 配置与主题：顶层 `mode` 单键（system/light/dark，兼容旧 `[theme] mode` 段）、系统外观跟随、深色标题栏/Mica、主题切换持久化（ADR-007）

明确排除（非目标，不做故障注入）：Mermaid、表格单元格内 inline 编辑、CRDT 协作、插件系统。

## 5. 外部输入点（Skill 02 B1）

1. 键盘/IME 事件（winit `Ime::Preedit/Commit`，自绘 preedit）
2. 鼠标/触控板（点击、选择、滚轮）与拖放文件
3. 打开的 `.md` 文件（任意来源、任意编码、任意大小）
4. 剪贴板（纯文本、HTML、位图）
5. 文件系统监听（当前文档被外部修改/删除）
6. CLI 参数（启动时的文件路径）
7. 配置文件（主题/快捷键，TOML）
8. 文档引用的图片文件（相对路径解析）

v1 无网络输入点（无同步、无更新检查、无遥测）。

## 6. 供应链工具（Skill 02 B5）

- 首选：GitHub Dependabot 依赖漏洞告警（仓库 Settings → Security 开启）⏳ 仓库建立后开启
- 备选：`cargo audit`（CI job，发布前必跑，无高危才放行）
- 直接依赖预算与理由：
  - `veil-core` ≤ 3：rope 实现 `lapce-xi-rope`（ADR-001 已定，含 vendor 退路）、解析手写（GFM 增量解析无现成轮子，不引入 pulldown-cmark 进内核）。grapheme 导航由 rope 内置（`prev/next_grapheme_offset`），原计划的 `unicode-segmentation` 已删除（2026-10-24 简化）
  - `veil-egui`：≤ 5。切片 1（2026-10-24）：`winit`（直收 IME，spike 验证路径）、`softbuffer`（软件呈现）、`cosmic-text`（排版）。切片 15 推翻了「chrome 落地引入 egui/GL」的原计划：菜单/状态栏/滚动条用 `Frame` 原语自绘，文件对话框走产品层手写 Win32 FFI，直接依赖保持 3（`winit`/`softbuffer`/`cosmic-text`）——见 ADR-005
  - `veil`：`pulldown-cmark`（仅 HTML 导出）、`notify`（文件监听）、`arboard`（剪贴板：文本/HTML/位图，`image-data` 属默认特性）、`latex-rust`（公式渲染，ADR-002 已定：KaTeX/rquickjs 只出 HTML 无法在无 webview 像素管线下光栅化，latex-rust spike 实测 100 公式同步 21ms 占预算 7%，详见 ADR-002 spike 表二）、`png`（latex-rust PNG 字节解码为 RGBA 与粘贴图片编码落盘，latex-rust 本身不依赖 png crate，此为独立直接依赖）。配置不引入 `serde`/`toml`：手写极简 TOML 子集解析（ADR-007），直接依赖维持 8/8
- 传递依赖总数阈值：≤ 200（CI：`cargo tree --prefix none --depth 100 | sort -u | wc -l`）；2026-10-25 升级后实测 121（与 §2 同口径），余量充足。现存重复版本全部上游锁定、本项目不可动作，登记为已知状态：png 0.17/0.18 + bitflags 1.3/2 + miniz_oxide 0.8/0.9 ← latex-rust 3.0.1 钉 tiny-skia 0.11；skrifa 0.39/0.40/0.44 + read-fonts/font-types 各三版 ← latex-rust / cosmic-text(harfrust) / cosmic-text(swash) 三消费者各自锁定；smol_str 0.2/0.3 与 windows-sys 0.52/0.61 ← winit 0.30。收敛路径：上游（latex-rust/winit）发版后随版本升级自然消除
- 黑名单（任何 crate 引入即 CI 失败）：`tokio`、`wasmtime`、`webview2-com`/`wry` 等一切 webview（无 Chromium/WebView2 进程是产品级承诺）

## 7. 平台接缝实例（Skill 01 B3）

- 语言：Rust（stable，edition 2021）。机制：条件编译 + 同名同签名模块；落点 `crates/veil/src/platform/`。
- `veil-core` 无任何平台代码（纯逻辑， WASM 可编译性作为副产物验证项 ⏳）。
- 接缝项（上层零平台分支）：主题（`ThemeEnv`：`ConfigStore` 配置原子读写、`SystemTheme` 注册表 `AppsUseLightTheme`、`WindowEffects` DWM 深色标题栏 + Mica；非 Windows 分别退化为无配置/浅色/空实现）、窗口材质（Windows Mica）、系统主题跟随、剪贴板（文本/HTML/位图）、图片资产落盘（`AssetStore`）、文件对话框（`FileDialog`：Windows 手写 `GetOpenFileNameW/GetSaveFileNameW` FFI，其余平台恒取消）、文件监听（`FileWatcher`：产品层 notify 事件线程 + 非阻塞通道，监听父目录并按文件名过滤；非 Windows 若 notify 初始化失败则 watch 返回 Err 降级为提示）、IME（winit 事件透传，自绘编辑器不经 egui 输入抽象）。
- 决策记录：
  - [ADR-001](../docs/adr/ADR-001-rope-selection.md)：rope 选型 lapce-xi-rope；mark 语义自封装；初版有序 Vec + 全量 delta transform，sum-tree 留作性能退路
  - [ADR-002](../docs/adr/ADR-002-math-rendering.md)：公式渲染走 `latex-rust` 纯 Rust 路径（KaTeX/rquickjs 否决：HTML 输出无法在无 webview 管线下光栅化）；Mermaid 降级为非目标
  - [ADR-003](../docs/adr/ADR-003-mark-bias-affinity.md)：mark bias + 光标 affinity 规则表；Reveal 下按纯文本编辑
  - [ADR-004](../docs/adr/ADR-004-platform-seam.md)：平台接缝 = 条件编译同名模块，落点 `crates/veil/src/platform/`
  - [ADR-005](../docs/adr/ADR-005-self-drawn-chrome.md)：chrome 自绘（winit+softbuffer 不变）+ 系统文件对话框手写 Win32 FFI，不引入 egui/GL
  - [ADR-006](../docs/adr/ADR-006-external-file-watch.md)：外部文件变更只提示不自动重载；notify 写入通道、主线程 300ms 轮询、覆盖前才备份
  - [ADR-007](../docs/adr/ADR-007-config-theme.md)：手写 TOML 子集（不用 serde/toml，守住 8/8 预算）；`Theme` 唯一颜色来源；注册表系统外观 1s 轮询；DWM 深色标题栏 + Mica（内容区纯色兜底）；配置原子写；切换即失效颜色派生缓存
  - [ADR-008](../docs/adr/ADR-008-dependency-upgrade-policy.md)：依赖升级策略——切片完成后允许 minor/patch `cargo update`；major 单独立项、单一 commit、可回滚；升级引起的数字变化与 INSTANCE.md 同 commit 回写；黑名单与 8/8 预算不变

## 8. 违禁项清单（Skill 03 A3）

- 词级（CI grep 目标，对外文档 README/Release Notes 不得出现）：

<!-- CI-BANNED-WORDS-BEGIN -->
md-inline-core
xi-rope
OffsetMapper
Muya
sam
overlay
reveil
<!-- CI-BANNED-WORDS-END -->

- 概念级（发布前人工核对）：对外文档不得描述「内核/注解分离」「mark 自动跟随」等内部架构机制本身；可描述用户可感知的效果（「光标处展开标记」）。
- CI 现状：⏳ 待建（CI 未建立前该门视为失败；清单非空，建立时读取上方标记块）

## 9. 环境维度实例（Skill 03 A5）

- OS：Windows 11 23H2+（基线）；Windows 10 22H2（次要，标「半」）；macOS/Linux（不承诺，标「半」）
- 运行时/引擎：无外部运行时（静态链接 MSVC）；softbuffer 软件呈现 + 自绘 chrome，无需 GPU/OpenGL；系统文件对话框由 comdlg32/user32 提供（Win32 系统组件，随 Windows 存在）
- 配置：区域=中文(简体)/英文(美国) 双测；DPI 100%/150%/200%（Per-Monitor V2 必须声明，否则高分屏模糊视为缺陷）；系统深色/浅色双测
- 安全软件：Defender 受控文件夹访问可能拦截文档目录写入——保存失败须给可读错误而非崩溃
- 权限：普通用户运行，禁止要求管理员
- 已知环境事实（易误判为缺陷，逐条列出）：
  1. 首次启动 cosmic-text/fontdb 扫描系统字体目录实测 107.8 ms（冷进程 145 ms），首个标题 shaping +7.9 ms、首个段落 +29.7 ms；冷启动红线（§2）超标的其余部分来自进程加载与 Defender 对新建 exe 的实时扫描（最小 414 ms / 中位 771 ms），已决策如实记录为平台成本，不设缓解项
  2. IME 预编辑串（preedit）以下划线样式显示是正常交互，不是渲染缺陷；缺陷的判定标准是候选框位置漂移或提交文本错位
  3. Defender 实时扫描可能使首次保存/图片落盘出现一次性延迟，重复执行幂等场景不受影响
