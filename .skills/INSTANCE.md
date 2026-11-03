# INSTANCE.md · 项目实例文件（Veil）

> 本文件是 ai-dev-skills 通用方法论在本项目的全部实例化：项目专有事实（命令、红线、工具名、平台名、专有清单）只存在于此，skills 本体不内嵌。
> 有效期：下列任一事实变更时需更新
> 最后验证：2026-10-31（**G3 门禁通过·自动化口径**：阶段 2 切片 2a–2f 全部收口，自动化红线全灭——782 测试全绿、clippy 零警告、e2e 41 断言、p95 6.65/12.93ms、体积 5.48MiB、传递依赖 141/200、CommonMark A/C/D 652、GFM 23/23；2d 落地 CJK 标点禁则（UAX #14 + WJ 胶水段补中文特有禁则）+ 满行悬挂标点 + IME preedit 同套排版规则。真机 P4 三项、IME 候选框跟随复核、盲测为人工项，转阶段 3 走查清单）；**阶段 3 切片 3a ✅**：冷启动环境段根因定位为火绒 HIPS 信誉拦截（非产品代码），稳定路径实测墙钟 33–79ms + 首帧段 ≈155–203ms，进 300ms，零代码改动；**阶段 3 切片 3b ✅**：打印/PDF（分页引擎 + 干净帧 + Win32 GDI 手写 FFI 零新增依赖，PDF 走 Microsoft Print to PDF 同码路）；**阶段 3 切片 3c ✅**：UX 打磨三项——Ctrl+B/I/K 行内格式、用户缩放（dpi_scale × user_zoom 0.1 量化 [0.5,3.0]，Ctrl+滚轮/捏合/±/0 + 视图菜单，打印不随用户缩放）、F11 无边框全屏（Esc 不退）；打印快捷键决策：Ctrl+P 归快速打开（VS Code/Typora 同），打印菜单可达不另占键；**阶段 3 切片 3d ✅**：原生单文件分发（用户决策不用 NSIS/MSIX/任何安装器外部工具链）——发布构建 GUI 子系统无控制台窗口，`crates/veil/build.rs` 由 workspace version 注入 VERSIONINFO（SDK rc.exe，零新增 crate 依赖，缺失降级跳过），PE subsystem=2、体积 5.51 MiB，版本号三方一致性红线可验证；workspace 805 测试绿、clippy 零警告；真机打印走查与 3c 交互目检转 3e 人工项

## 0. 项目骨架事实

- Workspace 三 crate，命名定型：`veil-core`（纯逻辑内核，无 GUI/异步依赖）、`veil-ui`（winit + softbuffer + cosmic-text 视图适配器，自绘 chrome，无 GL/egui）、`veil`（产品层二进制）。
- 依赖方向单向：`veil` → `veil-ui` → `veil-core`，禁止反向。
- 目录：`crates/veil-core`、`crates/veil-ui`、`crates/veil`。
- v1 目标平台：Windows 桌面。macOS/Linux 不承诺，验证时按 Skill 03 A1 标「半」。
- 已钉死的决策（详见 §7 ADR 清单）：
  - Markdown 源码是唯一真值；Reveal 等注解不持久化。
  - 内核单线程主编辑模型；后台任务（公式渲染）经消息队列回主线程，禁止跨线程改内核。
  - 内核零 GUI、零异步运行时依赖。
  - 非目标：wgpu 视图、协作/CRDT（位置模型见 ADR-010，v2 立项）、插件系统、移动端、表格可视化编辑（v1 仅渲染+源码编辑）、Mermaid 内嵌渲染。
- 命名与事实对齐：`veil-egui` 于 2026-11-03（v1 发布前）更名为 `veil-ui`——该 crate 自 ADR-005 起已零 egui 依赖，旧名属历史遗留；改名只动 crate 名/路径/use 语句，分层与职责未变。

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

- 抽检：① 微软拼音连续输入 200 字，候选框跟随光标不漂移；② 光标进出 `**bold**` 标记展开/收起，光标视觉位置不跳；③ 10 万字文档滚动到底部编辑，无全文档卡顿；④ 列表符号 `•`/引用左竖条/分隔线随光标进出切换显隐，行高不跳变；⑤ 相对路径 PNG 在行内/列表内渲染且超版心宽时等比缩放，点击图片进入源码态，断链/远程 URL 降级 alt 文本不崩溃；⑥ 截图后 Ctrl+V：`<文档名>.assets/` 生成且插入 `![](相对路径)`，重复粘贴递增 `image-2.png` 不覆盖，草稿态拒绝并保持文档不变；⑦ 菜单栏「文件/编辑」点击展开与切换、菜单项快捷键提示与悬停高亮、Ctrl+O 系统打开对话框、Ctrl+Shift+S 另存为（成功后标题去脏标/基线切换、取消无副作用）、长文档滚动条拖动与轨道翻页、状态栏行列随光标实时更新、菜单打开时 Esc 关闭且不误编辑；⑧ 外部编辑器改写当前文件：标题/状态栏出现「外部修改」、脏文档自动保存挂起、Ctrl+S 先备份（`.backup-*`）再覆盖；外部删除文件：提示「文件缺失」、继续编辑后自动保存重建文件；⑨ 「视图」菜单切换跟随系统/浅色/深色即时生效且「当前」标记移动、重启后保持、`%APPDATA%\Veil\config.toml` 内容与选择一致、系统深色模式下标题栏变深、Mica 在 Win11 显示于边框区域；⑩（3b/3c 新增）Ctrl+滚轮与触控板捏合缩放档位清晰、状态栏百分比提示正确、Ctrl+0 精确还原 100%、缩放后滚动条与光标落点正确；F11 进出无边框全屏、Esc 只关菜单不退全屏；Ctrl+B/I/K 对选区包裹/再按解包裹、空选区插标记对连按两次取消、`**粗体**` 内按 Ctrl+I 不误拆；文件菜单「打印 / 导出 PDF…」→ Microsoft Print to PDF 出 PDF 目检（多页分页不截字、数学图/表格跨页硬切、页码范围打印）、物理打印机抽验一份。
- 最近全绿：2026-10-26 ADR-009 S1~S5 + S6 三阶段收尾：workspace 527 测试全绿（veil-core 98 单测 + 64 集成、veil-ui 341 + 5 ignored、veil 24）、clippy 零警告；S6 阶段 A 清零全部 dead_code（导出收敛遗留 6 项 + Autosave::idle）、阶段 B 引入 proptest 钉死两类性质（任意编辑×主题/DPI 序列下 memo 命中值恒等于强制重算值；任意序列后连续两次 render 逐像素同）、阶段 B+ 为 persist 状态机补两条性质（六条可观察不变量：status 优先级自洽 / hold 必有原因 / 自动保存绝不覆盖外部版本 / 手工保存后磁盘==基线 / 外部版本必落冲突副本且旧副本不被改写 / 连续轮询幂等；同序列重放投影完全一致）、阶段 C 将 App 顶层字段 26→10（WindowState / DocState / PersistState / InputState / UIState 五子结构，C5 cache+index 因 16 处同时借用结构性冲突保留顶层）；S3 将 7 个手工失效方法与 30 处调用点替换为因果键 memo；S4 拆分 render/present（paint() = render + present），幂等性断言连续两次 render 逐像素同，present 无 surface 时纯 no-op；paint 在 headless 下真实跑像素（win_size 兜底 800×600），性能探针口径修正为真实 render→present 路径；S1 新增 mode.rs；S2 新增 document/persist.rs；三域分离目录——document/（doc_state + block_index + persist）、view/（cache + layout/frame/style/table/highlight/outline/search）、chrome/（mod + paint + cache）；根级 app.rs/mode.rs/interaction.rs/paint.rs/app_tests.rs；veil-core syntax/{mod,model,block,inline}；每切片独立可 revert commit；历史状态：阶段二 P1a~P6（444 测试：击键 p95 13.25→4.07 ms、视口惰性排版、Frame 单缓冲帧合成 4.22→0.28 ms、shell.rs 3916 行首拆、配置单键、撤销分组、删除注解层）；场景表逐格仍待真机人工验证，上列 ①~⑨ 尤需人工

## 2. 数字红线（Skill 03 A2）

| 数字 | 红线/目标 | 实测（2026-10-31 G3 门禁） |
|---|---|---|
| 冷启动（发布构建，`main`→首帧完成；**验收口径**，2026-10-30 用户裁定） | ≤ 300 ms | **145–190 ms** ✅（分段探针：事件循环 6–9 / 建窗 29–31 / Surface 49–88 / 首帧 61–69+12–13 ms） |
| 冷启动（墙钟，进程创建→首帧可交互；**环境观察项，不参与验收**） | — | **2026-10-31 切片 3a 复测：start→主窗口句柄 33–79 ms，加应用内首帧段后真墙钟 ≈155–203 ms，进 300 ms** ✅。G2 期 450–529 ms 根因已定位：本机火绒 HIPS 对「路径+哈希未入信誉缓存」的二进制做进程创建拦截扫描（探针：%TEMP% 未签名空二进制 start→exit 中位 1366ms、微软签名 cmd 副本 541ms，而 veil.exe 稳定路径下 33ms）；每次重编译产出新哈希即触发重扫，属开发期现象，终端用户固定路径安装不受影响。复现：`tools/bench/diag-premain.ps1` |
| 10 万字文档击键→提交上屏延迟 p95（release 探针串行） | ≤ 16 ms | 6.65 ms（5,005 块）✅；12.93 ms（10,500 块）✅（2026-10-31 G3 复测，Wrap::WordOrGlyph 断行分析成本 ≈0.4ms 已计入） |
| 10 万字文档常驻内存（任务管理器私有工作集） | ≤ 150 MB | 21.55 MB ✅ |
| 安装产物体积（`veil.exe` 单文件发布构建） | ≤ 15 MiB | 5,825,536 B ≈ 5.56 MiB ✅（2026-11-03 改名 + 批次 3 首批 mark 测试后重打包实测；无安装器，单文件即分发产物） |
| workspace 测试数（含 veil-core + veil-ui + veil） | ≥ 80 | **852 passed / 0 failed / 10 ignored** ✅（2026-11-03 全量实测；口径为所有测试二进制之和；G3 时 782 → 3d 805 → 3e 812 → 改名+Ports 收敛后 849 → 批次 3 首批 mark 测试 +3 后 852） |
| e2e 断言数 | ≥ 20 | **41** ✅（2026-10-30 切片 2e：`app_tests.rs` 3 个 e2e_ 场景测试——写作闭环打开→编辑→保存→重开 / 表格导航+格内编辑+图片粘贴落盘 / 查找替换→崩溃恢复→保存→三次启动自清） |
| Markdown 往返一致率（无编辑时 serialize 输出 == 输入字节） | == 100% | 100% ✅ |
| `veil-core` 的 GUI/异步运行时依赖数 | == 0 | 0 ✅ |
| `veil-ui`+`veil` 直接依赖总数 | ≤ 8 | 8 ✅（winit/softbuffer/cosmic-text/arboard/pulldown-cmark/latex-rust/png/notify；WIC 走平台 FFI 零新增依赖） |
| workspace 传递依赖总数 | ≤ 200 | 141 ✅（2026-10-31 G3 复测，CI 口径 `cargo tree --prefix none --depth 100 \| sort -u`；较 G2 的 121 增加 20，为阶段 0–2 功能切片引入的 notify/png/image 等，余量充足） |
| CommonMark 0.31.2 一致性 | A≥85% / C≥90% | A=652/652、C=652/652、D=652/652 ✅ |
| GFM 扩展一致性（Tables/TaskList/Strikethrough） | A/C/D≥95% | A=23/23、C=23/23、D=23/23 ✅ |
| 3 分钟 soak 内存漂移 | ≤ 0 | -0.17 MB ✅（G1 实测；G2 未复跑，阶段 1 未触碰长驻资源路径） |

- 版本号一致性：git tag == workspace `Cargo.toml` version == 产物元数据（`veil.exe` 文件属性版本）——✅ 2026-10-31 切片 3d 起可验证：`crates/veil/build.rs` 由 workspace version 生成 VERSIONINFO 并经 SDK rc.exe 编译链接（零新增 crate 依赖，rc.exe 缺失时降级跳过并告警）；实测 FileVersion/ProductVersion=0.1.0。发布构建 GUI 子系统（无控制台窗口），debug 保留控制台；ICON 资源（16/32/48/64）同样由 build.rs 代码绘制生成（无二进制资产入库、无 zlib），Explorer/任务栏/标题栏均显示品牌图标
- 版本失配处置记录：无
- 测量方法注记：冷启动 = PowerShell `Start-Process` 后按进程 `StartTime` 计到主窗口句柄就绪，5 次取中位（含启动器与本机进程创建开销，方差大，故同时记录最小值）；2026-10-30 起**同时记录分段值**——`VEIL_BOOT_LOG` 门控探针输出 `main` / `event_loop` / `surface_ready` / `redraw_done` 各 epoch 毫秒，可把墙钟拆成「进程创建→main」与「main→首帧」两段（分段探针为临时诊断代码，测毕已回退）。击键 p95 = 进程内端到端探针 `perf_keystroke_path`（release 构建，**必须串行** `--test-threads=1`，并行会假性劣化）；内存 = 启动后 2 s 的 `PrivateMemorySize64`；体积 = `target/release/veil.exe` 字节数。

## 3. 核心不变量断言（Skill 03 A4）

- 派生不变量：`veil-ui`/`veil` 的一切文本变更必须经 `veil-core` 单一入口（`Session::apply`）；断言方式：代码评审 + `cargo tree -p veil-core` 依赖红线（§2）CI 门禁。
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
  - `veil-ui`：≤ 5。切片 1（2026-10-24）：`winit`（直收 IME，spike 验证路径）、`softbuffer`（软件呈现）、`cosmic-text`（排版）。切片 15 推翻了「chrome 落地引入 egui/GL」的原计划：菜单/状态栏/滚动条用 `Frame` 原语自绘，文件对话框走产品层手写 Win32 FFI，直接依赖保持 3（`winit`/`softbuffer`/`cosmic-text`）——见 ADR-005
  - `veil`：`pulldown-cmark`（仅 HTML 导出）、`notify`（文件监听）、`arboard`（剪贴板：文本/HTML/位图，`image-data` 属默认特性）、`latex-rust`（公式渲染，ADR-002 已定：KaTeX/rquickjs 只出 HTML 无法在无 webview 像素管线下光栅化，latex-rust spike 实测 100 公式同步 21ms 占预算 7%，详见 ADR-002 spike 表二）、`png`（latex-rust PNG 字节解码为 RGBA 与粘贴图片编码落盘，latex-rust 本身不依赖 png crate，此为独立直接依赖）。配置不引入 `serde`/`toml`：手写极简 TOML 子集解析（ADR-007），直接依赖维持 8/8
- 传递依赖总数阈值：≤ 200（CI：`cargo tree --prefix none --depth 100 | sort -u | wc -l`）；2026-10-25 升级后实测 121（与 §2 同口径），余量充足。现存重复版本全部上游锁定、本项目不可动作，登记为已知状态：png 0.17/0.18 + bitflags 1.3/2 + miniz_oxide 0.8/0.9 ← latex-rust 3.0.1 钉 tiny-skia 0.11；skrifa 0.39/0.40/0.44 + read-fonts/font-types 各三版 ← latex-rust / cosmic-text(harfrust) / cosmic-text(swash) 三消费者各自锁定；smol_str 0.2/0.3 与 windows-sys 0.52/0.61 ← winit 0.30。收敛路径：上游（latex-rust/winit）发版后随版本升级自然消除
- 黑名单（任何 crate 引入即 CI 失败）：`tokio`、`wasmtime`、`webview2-com`/`wry` 等一切 webview（无 Chromium/WebView2 进程是产品级承诺）

## 7. 平台接缝实例（Skill 01 B3）

- 语言：Rust（stable，edition 2021）。机制：条件编译 + 同名同签名模块；落点 `crates/veil/src/platform/`。
- `veil-core` 无任何平台代码（纯逻辑， WASM 可编译性作为副产物验证项 ⏳）。
- 接缝项（上层零平台分支）：主题（`ThemeEnv`：`ConfigStore` 配置原子读写、`SystemTheme` 注册表 `AppsUseLightTheme`、`WindowEffects` DWM 深色标题栏 + Mica；非 Windows 分别退化为无配置/浅色/空实现）、窗口材质（Windows Mica）、系统主题跟随、剪贴板（文本/HTML/位图）、图片资产落盘（`AssetStore`）、文件对话框（`FileDialog`：Windows 手写 `GetOpenFileNameW/GetSaveFileNameW` FFI，其余平台恒取消）、文件监听（`FileWatcher`：产品层 notify 事件线程 + 非阻塞通道，监听父目录并按文件名过滤；非 Windows 若 notify 初始化失败则 watch 返回 Err 降级为提示）、IME（winit 事件透传，自绘编辑器不经 egui 输入抽象）、打印（`Print`：veil-ui 分页引擎出干净帧，Windows 手写 `PrintDlgW`/GDI `SetDIBitsToDevice` FFI 贴打印机 DC，帧像素 XRGB 直接作 32bpp 顶向下 DIB 零转换；PDF 走系统 Microsoft Print to PDF 同码路；非 Windows 退化为提示不支持）。
- 决策记录：
  - [ADR-001](../docs/adr/ADR-001-rope-selection.md)：rope 选型 lapce-xi-rope；mark 语义自封装；初版有序 Vec + 全量 delta transform；2026-11-03 复核确认常驻 mark 恒为 2，并量化升级触发条件（≥1,000 个 / 变换 >5% 帧预算 / 内存 >1MB）
  - [ADR-002](../docs/adr/ADR-002-math-rendering.md)：公式渲染走 `latex-rust` 纯 Rust 路径（KaTeX/rquickjs 否决：HTML 输出无法在无 webview 管线下光栅化）；Mermaid 降级为非目标
  - [ADR-003](../docs/adr/ADR-003-mark-bias-affinity.md)：mark bias + 光标 affinity 规则表；Reveal 下按纯文本编辑
  - [ADR-004](../docs/adr/ADR-004-platform-seam.md)：平台接缝 = 条件编译同名模块，落点 `crates/veil/src/platform/`
  - [ADR-005](../docs/adr/ADR-005-self-drawn-chrome.md)：chrome 自绘（winit+softbuffer 不变）+ 系统文件对话框手写 Win32 FFI，不引入 egui/GL
  - [ADR-006](../docs/adr/ADR-006-external-file-watch.md)：外部文件变更只提示不自动重载；notify 写入通道、主线程 300ms 轮询、覆盖前才备份
  - [ADR-007](../docs/adr/ADR-007-config-theme.md)：手写 TOML 子集（不用 serde/toml，守住 8/8 预算）；`Theme` 唯一颜色来源；注册表系统外观 1s 轮询；DWM 深色标题栏 + Mica（内容区纯色兜底）；配置原子写；切换即失效颜色派生缓存
  - [ADR-008](../docs/adr/ADR-008-dependency-upgrade-policy.md)：依赖升级策略——切片完成后允许 minor/patch `cargo update`；major 单独立项、单一 commit、可回滚；升级引起的数字变化与 INSTANCE.md 同 commit 回写；黑名单与 8/8 预算不变
  - [ADR-009](../docs/adr/ADR-009-state-model.md)：状态模型与架构极致化——状态五分类（真值/输入/派生/端口/瞬态）与唯一派生公式；交互模式状态机；持久化二维正交 + 时间即输入；`BlockId` 新鲜度作内容因果键、取消手工失效；`render`/`present` 分离使渲染幂等可测；公共导出 49→≤20；迁移按 S1~S5 独立可回滚
  - [ADR-010](../docs/adr/ADR-010-collab-position-v2.md)：协作位置模型——v1 冻结（不扩 mark、不上 sum-tree、不提前 block-relative）；v2 立项时远程锚点必须走 CRDT anchored position（Yjs RelativePosition / Automerge Cursor 形态），明确否决 sum-tree 与 block-relative 充当跨端锚点

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
- 运行时/引擎：无外部运行时（静态链接 MSVC）；softbuffer 软件呈现 + 自绘 chrome，无需 GPU/OpenGL；系统文件对话框由 comdlg32/user32 提供、打印对话框与打印后台由 comdlg32/gdi32/spoolss 提供（Win32 系统组件，随 Windows 存在）
- 配置：区域=中文(简体)/英文(美国) 双测；DPI 100%/150%/200%（Per-Monitor V2 必须声明，否则高分屏模糊视为缺陷）；系统深色/浅色双测
- 安全软件：Defender 受控文件夹访问可能拦截文档目录写入——保存失败须给可读错误而非崩溃
- 权限：普通用户运行，禁止要求管理员
- 已知环境事实（易误判为缺陷，逐条列出）：
  1. 首次启动 cosmic-text/fontdb 扫描系统字体目录实测 107.8 ms（冷进程 145 ms），首个标题 shaping +7.9 ms、首个段落 +29.7 ms；冷启动红线（§2）超标的其余部分来自进程加载与 Defender 对新建 exe 的实时扫描（最小 414 ms / 中位 771 ms），已决策如实记录为平台成本，不设缓解项
  2. IME 预编辑串（preedit）以下划线样式显示是正常交互，不是渲染缺陷；缺陷的判定标准是候选框位置漂移或提交文本错位
  3. Defender 实时扫描可能使首次保存/图片落盘出现一次性延迟，重复执行幂等场景不受影响
