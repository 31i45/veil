# Veil 全维度质量审计（2026-11-27）

> 性质：只读审计｜基线：HEAD `10df927`（`docs(qa): evidence and deferred register`），版本 `3.0.2`（`Cargo.toml:12`）
> 范围：架构与代码质量、十一个质量维度、八条产品原则、P0/P1 修复清单
> 本文件是 `converge-perfect-writing-surface` Task 5.1–5.4 的产出；不修改源码、README、`.trae`，不运行 cargo，不提交。唯一新增文件为本文件。

## 0. 审计口径

### 0.1 事实来源（本次实际读取）

| 类别 | 文件 |
|---|---|
| 缺陷台账（权威清单） | `docs/qa/perfect-surface-audit-2026-11-27.md`（F-001…F-034） |
| 十维评分 | `docs/review/jobs-lens-report-v2.md`（41/50，含 2026-11-26 追加节） |
| 跨表面证据 | `docs/review/evidence/layout-consistency-2026-11-26.md` |
| 交付事实 | `docs/qa/release-facts-2026-11-27.md`、`docs/qa/w11-load-state-measurement.md`、`docs/qa/w10-output-pipeline-evidence.md` |
| ADR | `docs/adr/ADR-004-platform-seam.md`、`ADR-009-state-model.md`、`ADR-011-wgpu-deferral.md`、`ADR-012-syntax-highlighting.md` |
| 产品面 | `README.md`、`README.zh-CN.md` |
| 任务边界 | `.trae/specs/converge-perfect-writing-surface/tasks.md` |
| 源码 | `crates/veil-core`、`crates/veil-ui`、`crates/veil`、`crates/veil-mermaid-offline`（Grep/Read 抽查 + 行数/标记统计） |

### 0.2 证据等级

- **E1**：源码/配置/文档可定位到文件与行号。
- **E2**：仓库内已有实测记录（`docs/qa/*`、`docs/review/evidence/*`），本次未复跑。
- **E3**：由代码结构推断，文中显式标注"推断"。
- 无法证实的一律写"待验证"，不改写为结论；不重复抄台账正文，只引用 F 编号并补充架构/原则视角。

### 0.3 与台账的关系

台账 34 条（P0 0 / P1 15 / P2 14 / P3 5）是**缺陷**清单；本文件不重述其内容，只回答三个台账未覆盖的问题：① 这些缺陷背后的结构成因是什么；② 十一个维度与八条原则的现状如何；③ 哪些必须现在修（P0/P1）以及怎么验证。台账中归因为"本轮修复"的 F-005…F-009 在 `tasks.md:19-24`（Task 3）已勾选关闭，本文件按"已关闭、需回归看护"处理，不再计入 P1。

---

## 1. 架构与代码质量

### 1.1 组件边界与接缝：清晰，可替换，可 headless 测试

**事实**：平台能力全部以 trait 端口定义在 `veil-ui`，由产品层 `veil` 注入；`veil-ui` 不含任何具体系统 API。

| 端口 | 定义位置 | 产品层实现 | 可 headless |
|---|---|---|---|
| `Clipboard` | `crates/veil-ui/src/app.rs:62` | `arboard` 注入（`veil/Cargo.toml:17`） | 是 |
| `FileStore` | `app.rs:85` | std fs 原子写 | 是 |
| `Clock` | `app.rs:98` | Win32 `GetLocalTime`（`platform/clock.rs`） | 是（注入固定时钟） |
| `FileDialog` | `app.rs:134` | `platform/windows.rs` Win32 对话框 | 是（默认 None） |
| `FileWatcher` | `app.rs:168` | `platform/watch.rs`（notify） | 是 |
| `Exporter` | `app.rs:177` | `crates/veil/src/export.rs` | 是 |
| `UrlOpener` | `app.rs:194` | `platform/shell.rs` ShellExecuteW | 是 |
| `ImeAnchor` | `app.rs:203` | `platform/ime.rs` Imm32 | 是（`NoopImeAnchor`，`app.rs:208`） |
| `RecoveryStore` / `SessionStore` / `SnapshotStore` | `app.rs:278` / `:286` / `:310` | `platform/mod.rs:809` `persist_env()` | 是 |
| `Print` | `crates/veil-ui/src/print.rs:26` | `platform/print.rs` GDI | 是（`NoPrint`，`print.rs:38`） |
| `MathRenderer` | `crates/veil-ui/src/math.rs:19` | `latex-rust` | 是 |
| `MermaidRenderer` | `crates/veil-ui/src/mermaid_renderer.rs:30` | `veil-mermaid-offline`（QuickJS） | 是 |
| `ImageLoader` / `AssetStore` | `veil-ui/src/image.rs` | WIC + png | 是 |
| `TextMetrics` | `crates/veil/src/platform/pdf.rs:145` | 字体 cmap 度量 | 是（`FakeMetrics`） |
| `MediaProvider` | `crates/veil/src/platform/pdf_layout.rs:105` | WIC / latex-rust | 是 |

**判断**：接缝设计是本项目最强的结构性资产。所有端口都在 `veil-ui` 侧定义、在 `veil` 侧实现（依赖倒置），`Ports` 聚合见 `app.rs:373-391`；`veil-ui` lib 测试 1466 项（`release-facts-2026-11-27.md:13`，E2）证明 headless 路径真实可测，而非"理论上可替换"。`Print`/`Exporter` 都通过 `PrintSession::notice_error`（`print.rs:202`）把失败回灌为用户可见通知，而不是静默返回。

**不足**：① 端口签名统一使用 `Result<_, String>`（全仓 227 处，如 `app.rs:86-87`、`print.rs:26`），错误归一为字符串，调用方无法按类别分支，只能透传给 UI；② `Print::print` 无返回值（`print.rs:27`），错误必须经由 `PrintSession` 的副作用通道传递，契约不在类型里；③ `veil-ui/src/qa.rs`（`lib.rs:43` 导出）与 `veil/src/qa.rs`（`veil/src/lib.rs:10-12`，`#[doc(hidden)]`）是跨 crate 测试门面，随产品库一起编译与导出。

**严重度**：接缝本身 P3（良好）；`Result<_, String>` P2（可维护性）；`qa` 门面 P3。

### 1.2 层间依赖方向：单向，无环

**事实**：`veil-core` 仅依赖 `lapce-xi-rope` + `serde_json`（`crates/veil-core/Cargo.toml:7-14`）；`veil-ui` 依赖 `veil-core` + winit/softbuffer/cosmic-text/syntect（`crates/veil-ui/Cargo.toml:7-28`）；`veil` 依赖 `veil-core` + `veil-ui` + 可选 `veil-mermaid-offline`（`crates/veil/Cargo.toml:12-16`）；`veil-mermaid-offline` 为叶子 crate。

**判断**：依赖方向严格单向 `core ← ui ← product`，`veil-core` 零平台代码（`README.md:225` 记录 GUI/async 依赖 0，E2）。这是"可移植性只保 core"的结构前提。ADR-004 要求"零 `#[cfg]` 出现在 platform/ 之外"（`docs/adr/ADR-004-platform-seam.md:21`），但 `veil-ui/src/app.rs:14-15`、`:215-226` 仍有 `#[cfg(windows)]`（取 winit 的 HWND 交给 `ImeAnchor`）。这是**必要的物理事实外泄**，但确实是 ADR 原文与代码的偏差。

**严重度**：方向 P3（良好）；ADR-004 偏差 P3。

### 1.3 自研 vs 外购：自研集中在"必须掌握字节与几何"的地方

| 能力 | 选型 | 判断 |
|---|---|---|
| 块/行内解析（`veil-core/src/syntax/block.rs` 4114 行、`inline.rs` 3150 行） | 自研 | **必要**：编辑器需要字节级 `Range`、可逆序列化、光标/标记锚点（ADR-001/ADR-003），外部解析器不提供源码区间契约 |
| 断行/禁则（`pdf_layout.rs:198-357`、`veil-ui/src/view/linebreak.rs`） | 自研 | **必要**：`unicode-linebreak` 已评估为 NO-GO（`jobs-lens-report-v2.md:204`、`README.zh-CN.md:155`），行为契约（`LItem`/`last_space`）无法被替换 |
| PDF 版面/分页（`pdf_layout.rs` 4196 行） | 自研于 `pdf-writer` 之上 | **合理**：分页是产品差异点，且与屏幕共用排版语义；底层写对象复用 `pdf-writer`/`subsetter`（`veil/Cargo.toml:31-33`） |
| 表格几何（`view/table_strategy.rs`） | 自研 | **合理**：与光标落点、公式占位强耦合 |
| 自绘 chrome + 软件呈现 | 自研（ADR-005） | **合理**：零 GL 上下文、真实命中/像素可测 |
| 文本 shaping | `cosmic-text` | **正确复用** |
| 代码高亮 | `syntect`（ADR-012） | **正确复用**：删除 1000+ 行自研状态机，覆盖 19→75 语言 |
| **HTML 导出解析** | `pulldown-cmark`（`export.rs:143-145`、`:339`） | **第二套解析器**：与 `veil-core` 并行，是最大的结构性重复 |
| Mermaid | QuickJS + 完整 `mermaid.js` + `resvg` | **合理**：离线契约下无可替代（`README.zh-CN.md:134`） |

**判断**：自研边界划在"必须掌握字节与几何"的地方，复用边界划在"通用图形/语法/IO"的地方，方向正确。唯一的结构性重复是 HTML 导出走 `pulldown-cmark` 重新解析，而屏幕/PDF 走 `veil-core` AST。仓库已用差分 oracle 把风险变成可测债：721 篇语料、块骨架差异 7 篇、渲染差异 28 篇、需修 0（`jobs-lens-report-v2.md:197`，E2）。

**取舍结论**：保持现状，不合并解析器。理由是 HTML 导出的目标是"标准 CommonMark/GFM 输出"，自研 AST 序列化到 HTML 需要另写一套等价规则，成本高于维护差分测试；但必须把差分测试当作**长期门禁**，而不是一次性 spike。

**严重度**：双解析器 P2（有测量、可接受）；选型总体 P3（良好）。

### 1.4 错误处理：生产路径几乎无裸 unwrap，但 panic=abort 放大了不变式违例的代价

**事实（全仓统计，`Grep`/`Select-String` 实测）**：

| 标记 | 次数 | 说明 |
|---|---:|---|
| `.unwrap()` | 1053 | 绝大多数在 `#[cfg(test)]` 与 `tests/` |
| `.expect(` | 599 | 同上 |
| `panic!(` | 111 | 同上 |
| `unsafe` | 136 | 集中在 `platform/`（`print.rs` 39、`windows.rs` 19、`http.rs` 17、`ipc.rs` 12、`wic.rs` 12、`assoc.rs` 11），`veil-ui/shell.rs` 7 |
| `#[allow(dead_code)]` | 17 | 见 §1.5 |
| `TODO` / `FIXME` | 5 / 36 | **逐条查看均为测试语料**（`[!FIXME]` Callout、`- [ ] todo`、mermaid corpus 的 `todo` 节点），**无真实债务标记** |

**生产段核对**（按测试模块边界切分后）：

- `veil-mermaid-offline/src/lib.rs` 生产段（1–357 行）0 处 unwrap/expect；测试模块从 `:358` 起。
- `veil-core/src/session.rs` 测试模块从 `:380` 起，生产段 0 处。
- `veil-ui/src/view/table_strategy.rs` 测试模块从 `:953` 起，生产段 0 处。
- `veil-ui/src/editor.rs` 生产段 20 处，全部为"恒合法"不变式（如 `:100` "空文档 offset 0 恒合法"、`:517` "选区恒合法"），测试模块从 `:1335` 起。
- `veil/src/platform/pdf.rs:1261` `children[i].last().unwrap()` 受 `:1259` `!is_empty()` 守卫，安全。

**用户可见失败路径**（E1）：

- 打开失败按 `NotFound`/`Denied`/`NotUtf8`/`Other` 归因并含"下一步"文案（`platform/error.rs:11-42`、`:47-60`）；启动失败以空白草稿开窗并弹模态（`veil-ui/src/app.rs:336-354`）。
- 保存/导出/打印失败走 `notice_error`（`print.rs:202-232`），错误通知不自动消退（`app.rs:57-59`）。
- 落盘全部"tmp + rename"原子写（`platform/mod.rs:229-251`），失败回清理临时文件（`:221-227`）。
- 崩溃取证：panic hook 最早安装（`veil/src/lib.rs:42`，`platform/diag.rs`）。

**判断**：错误处理的主体是"类型化归因 + 用户可见文案 + 原子写"，质量高于一般 Rust GUI 项目。风险在两点：① release 为 `panic = "abort"`（`Cargo.toml:17`），`editor.rs` 的 20 处不变式 `expect` 一旦触发即为进程级退出，只能靠 diag 取证——这是**有意取舍**（宁可崩也不写错字节），但需要承认它把"内部 bug → 可见崩溃"的路径保留了下来；② `Result<_, String>` 让"可重试/不可重试"只能由调用方按字符串判断。

**严重度**：整体 P3（良好）；不变式 panic + abort P2（已记录、可接受）；stringly 错误 P2。

### 1.5 死代码 / 重复 / 耦合：无幽灵代码，有巨型模块

**事实**：

- `#[allow(dead_code)]` 17 处，逐处带理由且均有测试消费：`editor.rs:87`（测试用构造器）、`view/layout.rs:506`（`STACK_ORDER` 顺序契约）、`tokens.rs:11/109/120/253/268`（alpha 阶梯与阴影表）、`theme.rs:269/401/411`（登记表）、`veil-core/src/syntax/model.rs:829`（`HTML_INLINE_WHITELIST`）、`veil/src/platform/windows.rs:28`。**属于"为测试/契约保留"的声明式死代码，不是腐化。**
- 大文件（>1500 行）共 25 个，其中**产品代码 16 个**：`pdf_layout.rs` 4196、`veil-core/syntax/block.rs` 4114、`export.rs` 3635、`inline.rs` 3150、`interaction.rs` 2982、`view/settings.rs` 2795、`theme.rs` 2764、`editor.rs` 2747、`app.rs` 2728、`i18n.rs` 2678、`view/layout.rs` 2636、`paint.rs` 2264、`veil-mermaid-offline/lib.rs` 2142、`syntax/mod.rs` 1967、`platform/pdf.rs` 1965、`view/table_strategy.rs` 1703；其余 9 个为测试文件（`syntax_matrix.rs` 3119、`app_tests/*` 等）。
- `App` 顶层字段 **18 个**（`app.rs:895-928`，含 `doc/config/callouts/window_state/editor/engine/index/cache/pending_mermaid_auto/mermaid_auto_requested/input_state/ports/persist_state/ui_state/cursor_visible/last_blink/last_seen_caret/deferred_startup`），ADR-009 的终态目标为 **≤12**（`ADR-009-state-model.md:183`）。
- 耦合点：`i18n.rs` 2678 行集中全部中英文案，`interaction.rs` 2982 行集中全部命令执行。

**判断**：真正的"死代码"极少，说明代码卫生好；但**巨型模块**是主要可维护性成本：`pdf_layout.rs` 单文件同时承载 `PageGeom`/`LayoutOpts`/`LinkAnnot`/`DocLayout`/`Ctx` 版面引擎（`:28`…`:386`…`:2394`）与约 1800 行测试；`block.rs` 与 `inline.rs` 同理。ADR-009 明确反对"为拆分而拆文件"（ADR-009 §七），所以不应把行数本身当缺陷；但 4000 行文件已超过单次评审的可读上限，建议按**职责**（版面计算 / 字符收集 / 输出编码）拆为子模块，而非按行数切。

**严重度**：死代码 P3；巨型文件 P2；`App` 字段 18 > 目标 12 P3。

### 1.6 §1 汇总

| 维度 | 结论 | 主要证据 | 严重度 |
|---|---|---|---|
| 端口/接缝 | 好 | 15 个 trait + `Ports`（`app.rs:373-391`） | P3 |
| 依赖方向 | 好（ADR-004 有偏差） | `Cargo.toml` × 4；`app.rs:14-15/215-226` | P3 |
| 自研/外购 | 好（双解析器为已知债） | §1.3 表；`export.rs:143-145` | P2 |
| 错误处理 | 好（stringly + abort 有代价） | 统计表；`platform/error.rs:11-60` | P2 |
| 死代码/重复 | 好 | 17 处 `allow(dead_code)` 逐条有理由 | P3 |
| 模块体积/耦合 | 不足 | 16 个产品文件 >1500 行；`App` 18 字段 | P2 |

---

## 2. 十一个质量维度

> 每维：现状事实 → 证据 → 结论 → 严重度 → 改进建议。性能/兼容数值沿用仓库既有实测（E2），本次未复跑。

### 2.1 UX（体验与交互）— 可接受

- **事实**：命令面板覆盖 39 个非分隔线菜单项（`jobs-lens-report-v2.md:105`，E2）；`Ctrl+P` 快速打开、`Ctrl+Alt+P` 打印、`Ctrl+Shift+H` 历史、`Ctrl+Shift+O` 大纲、`Ctrl+,` 设置；错误通知不自动消退（`app.rs:57-59`）；模式路由为纯函数且可穷举（`mode.rs:85`，`:211`）。
- **证据**：`mode.rs:92-183`（菜单/快速打开/查找/历史/文档五模式转移表）、`app_tests` 1466 项。
- **结论**：交互语义清晰、焦点闭环完整；扣分点是**主路径 IME 的真机行为仍未验证**（F-014），而中文写作工具里 IME 属于主路径。
- **严重度**：P1（证据缺口，非代码缺陷）。
- **建议**：按 `deferred-and-manual-register-2026-11-27.md` §1.3 完成六步走查；在此之前 README 保持 NOT RUN。

### 2.2 审美（视觉与表达）— 好

- **事实**：版心按 40 全角封顶、窄窗铺满（`geometry.rs:126-142`，无 1600px 门槛）；标题字号同源于 `type_scale`（`release-facts:53`）；浅色 7 个语法 token 全部 ≥4.5:1、深色标点 4.78:1（`release-facts:52`）；alpha 阶梯单源（`tokens.rs:11`）。
- **证据**：`geometry.rs:136-141`；`tokens.rs`；`theme.rs:557-560`（引用自 jobs-lens）。
- **结论**：视觉系统有单一色板来源与可断言的对比度，属"可作为产品主张"的水平。
- **严重度**：P2（文档漂移，见 §4 P2-1）。
- **建议**：修 README 的"1600px"表述（`README.md:109`、`README.zh-CN.md:109`），改成"可用宽度超过约 40 全角时封顶并居中"。

### 2.3 功能适用性 — 可接受

- **事实**：语法矩阵 859 用例 / 6013 断言 / 0 FAIL / 0 NOT RUN（`release-facts:28`，E2）；七表面一致；但块级单换行、行内换行/标点边界、容器内相邻、嵌套容器四类组合尚未覆盖（F-001…F-004，Task 2 未完成，`tasks.md:11-17`）；宽表触底后向右溢出被裁剪（F-010，`table_strategy.rs:8-9` 原文"v1 不做横向滚动"）。
- **证据**：`report-2026-11-27.md:106-117`；`table_strategy.rs:9`、`:33`（`MIN_COL_WIDTH = 40.0`）。
- **结论**：主路径功能完整，缺口集中在"组合边界"与"超出版心的复杂内容"。
- **严重度**：P1（F-001…F-004、F-010）。
- **建议**：Task 2 与 Task 4.1 继续，不在本轮审计内实施。

### 2.4 性能效率 — 好

- **事实**：端到端首帧 84.8 ms（5 次中位，PASS）、击键内核 p99 0.305 ms / p95 0.109 ms（PASS）、峰值 RSS 52.6 MB（PASS）、exe 14,291,456 B（门禁 14,680,064 B，余量 388,608 B）（`jobs-lens-report-v2.md:129`、`release-facts:23`，E2）；5005 块全量排版 281.10 ms 已改称"合成参考指标"，真实脏块重排 271 µs（`jobs-lens-report-v2.md:344-346`，E2）。
- **证据**：同上；`w11-load-state-measurement.md:42-46`（5000 文件快速打开 p95 12.885 ms、大纲 0.344 ms）。
- **结论**：交互路径全部达标，唯一"看似失败"的全量排版指的是合成基准而非用户可感路径，口径已被诚实更正。
- **严重度**：P3。
- **建议**：保持现有口径；若未来重开门禁，先做 shaping 复用的独立项目并给 p95。

### 2.5 兼容性 — 不足

- **事实**：Windows-only 产品（`README.md:172`）；高对比度真实系统切换 PASS（228 ms，`jobs-lens-report-v2.md:145`）；user_zoom 四档 PASS；系统 DPI 150% PASS；**DPI 100/125/200、副屏、物理打印机、真机中文 IME 全部 NOT RUN**（F-012/F-013/F-014/F-025）；AccessKit/UIA 层已于 `3a6d72d` 永久移除，但 README 仍宣称存在（见 §4 P1-1）。
- **证据**：`README.zh-CN.md:154`；`jobs-lens-report-v2.md:145`、`:270`；`git show --stat 3a6d72d`（删除 `accessibility.rs` 666 行）。
- **结论**：兼容性不是"代码没写"，而是"证据没跑"+ "文档与代码相反"。
- **严重度**：P1。
- **建议**：Task 6.1/6.2/6.3 的人工项按登记表执行；无障碍部分先修文档，再决定是否重新立项。

### 2.6 易用性 — 好

- **事实**：5000 文件快速打开约 13 ms（`w11:64`）、大纲 0.34 ms；设置面板 14 个字段（13 可编辑 + 1 只读窗口几何，`settings.rs:206-304`）；空态/异常四态矩阵 44 格成文（`release-facts:56`）。
- **证据**：`w11-load-state-measurement.md:60-66`；`settings.rs:206-304`。
- **结论**：日常路径无可感知摩擦；扣分点是 REAME 写"13 项（12 个可编辑）"（`README.zh-CN.md:114`）与实际 14/13 不符。
- **严重度**：P3。
- **建议**：同步 README 计数，或改为"不列数量"避免再次漂移。

### 2.7 可靠性 — 好

- **事实**：原子写 + 失败清理（`platform/mod.rs:229-251`）；崩溃恢复单槽（`persist_env`，`:809-824`）；快照按 hash 校验、损坏跳过不崩（`:739-757` 测试）；持久化外部变更以 proptest 钉死六条不变量（ADR-009 §3.5，`docs/adr/ADR-009-state-model.md:89`）；panic hook 最早安装（`veil/src/lib.rs:42`）。
- **证据**：同上；`platform/mod.rs:426-432`（非 UTF-8 / 不可读快照跳过）。
- **结论**："字不会丢"有结构性保证，并有性质级测试兜底。
- **严重度**：P3。
- **建议**：`editor.rs` 的不变式 `expect` 可考虑在关键路径降级为"可恢复错误 + 状态栏提示"，但这是取舍，不是缺陷。

### 2.8 安全性 — 好

- **事实**：链接协议白名单 `http/https/mailto/tel`（`app.rs:233-238`）；远程图片默认关；抓取拒绝 localhost/回环/私有/链路本地（`http.rs:193`、`:225-236`），限 3 次重定向、拒绝 https→http 降级（`http.rs:304-313`）；QuickJS 设 `set_memory_limit`、`set_max_stack_size(4 MiB)` 与中断 deadline（`veil-mermaid-offline/src/lib.rs:116-126`、`:232-233`）；无遥测/无后台网络（`README.zh-CN.md:136`）；SECURITY 三态披露三项未验证（`README.zh-CN.md:150`）。
- **证据**：同上。
- **结论**：默认值站在用户一侧，且"不安全默认"被结构性关闭；未验证项已被书面披露，不冒充通过。
- **严重度**：P2（未验证的 peer 核验/DNS rebinding/资源上限恶意实测）。
- **建议**：保持远程图片默认关；把三项未验证登记到发布检查单，而非反复写进 README。

### 2.9 可维护性 — 可接受

- **事实**：2251 测试通过 / 17 ignored（`release-facts:12`，E2）；clippy `-D warnings` 0 警告、fmt 通过（`:18-19`）；14 份 ADR 覆盖关键决策；双解析器差分、配置 round-trip、断行差分都有可重跑测试。反面：16 个产品文件 >1500 行、227 处 `Result<_, String>`、`App` 18 字段（目标 ≤12）、17 处 `allow(dead_code)` 需要持续人工判断。
- **证据**：§1.4/§1.5 统计；`ADR-009-state-model.md:183`。
- **结论**：测试与文档纪律强，模块粒度与错误类型是主要债。
- **严重度**：P2。
- **建议**：下一轮只做两件低风险改进——`pdf_layout.rs` 按职责拆子模块；把 `print`/`export` 的错误类型化（`enum PrintError`），保持行为不变。

### 2.10 可移植性 — 可接受（设计内）

- **事实**：`veil-core` 零平台代码、纳入 Linux CI（`README.md:172-173`）；`veil`/`veil-ui` 的 Windows 专属能力集中在 `platform/`（`cfg(windows)` 见 `platform/mod.rs:10-33`），非 Windows 有安全兜底（`:130-149`）；ADR-004 规定的 `macos.rs`/`linux.rs`"未实现即编译失败"未落地，改用"兜底实现"。
- **证据**：`platform/mod.rs:130-149`；`ADR-004-platform-seam.md:21-25`。
- **结论**：按"Windows-only 产品 + core 可移植"的定位是自洽的；ADR 与实现的差异是文档债。
- **严重度**：P3。
- **建议**：把 ADR-004 的"同名模块编译失败"改写为"非 Windows 兜底 + 编译整洁"，或真的补桩模块，二者选一，避免决策文档失真。

### 2.11 小结分布

| 结论 | 维度 |
|---|---|
| 好 | 审美、性能效率、易用性、可靠性、安全性（5） |
| 可接受 | UX、功能适用性、可维护性、可移植性（4） |
| 不足 | 兼容性（1） |

代码质量单列于 §1（结论：好，P2 残留：巨型模块、stringly 错误、18 字段）。

---

## 3. 八条产品原则

### 3.1 第一性原理

- **正面**：先定义"什么是不可协商的"再写代码——离线契约（`README.zh-CN.md:136`）、字节可逆（`docio`/BOM 还原，`app.rs:361-362`）、"字不会丢"（原子写 + 恢复）。ADR-011 用"慢在 CPU shaping，不在像素合成"这一事实否决 GPU（`ADR-011-wgpu-deferral.md:30`），是从第一性推导，不是从竞品推导。
- **反面**：`README.zh-CN.md:154` 与 `README.md:109` 仍有"1600px"这类与实现相反的第一性描述，说明"承诺"与"事实"之间缺少自动同步机制。
- **判断**：原则执行到位，但**文档层缺少与代码的校验回路**（P2）。

### 3.2 二八定律

- **正面**：字体度量、断行、分页、IME 锚点、崩溃恢复这五个 20% 的工作覆盖了"中文写作"80% 的痛感；5000 文件的快速打开/大纲实测 13 ms / 0.34 ms 后**关闭 G4、不引入异步加载态**（`w11:60-66`），是拒绝为长尾付成本的典型。
- **反面**：`mermaid.js` 完整打包带来约 5 MiB（`veil/Cargo.toml:8`）与 16 种图型未目检（F-011），即"为了 20% 的场景付出了 80% 的体积与验证面"。
- **判断**：主干判断正确；Mermaid 是已知的超配项，需要以"见证成本"而非"删功能"处理。

### 3.3 软件设计哲学（模块化 / 信息隐藏 / 接缝）

- **正面**：端口-适配器（§1.1）；`veil-core` 不暴露 rope 类型（`veil-core/Cargo.toml:8-9`）；ADR-009 把状态分成真值/输入/派生/端口/瞬态五类并给出唯一派生公式（`ADR-009-state-model.md:30-48`）；`CacheKey` 因果键让"忘调失效"结构性消失（§四）。
- **反面**：`App` 仍有 18 个顶层字段，且 `cursor_visible`/`last_blink`/`last_seen_caret` 三个瞬态未收进子结构（`app.rs:919-927`），与"五类登记"纪律不完全一致；`Result<_, String>` 让错误的信息隐藏变成了信息丢失。
- **判断**：结构纪律是本项目最强项，但收尾未完成（S6 未做）。

### 3.4 UNIX 哲学（组合 / 单一职责 / 文本流）

- **正面**：纯函数化的关键路径——`route(mode,key,mods,composing) -> Cmd`（`mode.rs:85`）、`compute_page_breaks`（`print.rs:357`）、`fill_lines`（`pdf_layout.rs:198`）、`content_span`（`geometry.rs:126`）；文本是真值（Markdown 源码），视图只是投影（`README.zh-CN.md:135`）；`/register`、`/quiet` 等 CLI 子命令（`veil/src/lib.rs:63-87`）。
- **反面**：`print_document` 用 `std::mem::replace` 置换端口来规避借用（`print.rs:319-324`），这是 Rust 借用限制下的权宜，不是干净的组合；`i18n.rs` 2678 行单文件集中全部文案，违背"小即美"。
- **判断**：函数级单一职责做得好；文件级与借用层面的组合仍有粗糙处（P2）。

### 3.5 少即是多

- **正面**：反目标 10 条明确排除协作/插件/WebView/富文本模型（`README.zh-CN.md:126-137`）；ADR-009 §七列出六个"明确不做"（ECS/actor/响应式/事件总线/DI/插件）；依赖预算传递 ≤200（实际 125，`README.md:227`）。
- **反面**：`mermaid.js` + resvg + QuickJS 是"少即是多"的最大例外；无障碍能力被移除后，README 仍保留一整节，属于"多写的文档"。
- **判断**：产品面克制，工程面偶有超配，但均有成文理由。

### 3.6 极简主义

- **正面**：单 exe、零 webview、零 GL 上下文（ADR-005/ADR-011）；首帧 84.8 ms、空闲零定时器（`README.zh-CN.md:112`"零定时器、零空闲唤醒"）；chrome 自绘不建图标系统（`release-facts:57`）。
- **反面**：`platform/` 手写 79 条 extern FFI（`jobs-lens-report-v2.md:206` 语境），换来的是"零新增依赖"，本质是用复杂度换体积，是"极简"的另一种成本。
- **判断**：极简是真实的工程约束，不是口号；代价集中在 FFI 维护面。

### 3.7 极致美学

- **正面**：40 全角版心 + 7:1 对比度 + 有尊严的降级（坏图 `[alt] + 占位框`，`jobs-lens-report-v2.md:113`）；`type_scale` 单一字阶（`release-facts:53`）；alpha 阶梯单源（`tokens.rs:11`）；空态用中性色（`release-facts:56`）。
- **反面**：`mark` 与公式失败配色、高对比度色板曾出现"第二套手写色值"，到 W7 才收敛（`release-facts:63`），说明"单一来源"是事后补的纪律而非先验约束。
- **判断**：审美水平高，但设计系统的一致性依赖审计，而不依赖类型系统。

### 3.8 极致 UX

- **正面**：错误通知不自动消退、外部修改四选一、崩溃恢复说明"对象/时刻/目录"（`jobs-lens-report-v2.md:105`）；焦点闭环与 Esc 归还正文有测试（`chrome_ui.rs:1378-1452`）；命令面板中文子串过滤可用。
- **反面**：中文 IME 真机、物理打印、副屏、DPI 变体仍未走通（第七维）；宽表在窄窗被裁剪且无横向滚动（F-010），这是"极致 UX"的直接反例——用户看得到内容却读不完。
- **判断**：失败语义与键盘路径接近"可作产品主张"，但"人体边界"未闭合，不能宣称极致。

---

## 4. P0/P1 清单（对接 Task 5.4）

### 4.1 P0

**0 条。** 本次审计未发现已证实的数据丢失、崩溃或安全问题：落盘为原子写、快照有 hash 校验、恢复路径有测试、远程抓取有 SSRF 防护。`editor.rs` 的不变式 `expect` 与 `panic=abort` 属"设计上宁可退出"的取舍，未被证实在真实输入下可触发，故不升级为 P0。

### 4.2 P1（6 条）

| ID | 问题 | 事实与证据 | 建议修复范围 | 验证方式 |
|---|---|---|---|---|
| **P1-1** | README 宣称的 AccessKit/UIA 无障碍能力**已不存在** | `3a6d72d refactor(ui)!: 永久移除无障碍（AccessKit）能力`（删 `crates/veil-ui/src/accessibility.rs` 666 行，`git show --stat`）；`Cargo.lock` 无 `accesskit`；`crates/**` 无 `accesskit/accessibility/UIA` 命中。但 `README.md:158-166`、`README.zh-CN.md:159-167` 仍写"`accessibility` feature，发行版默认开启"并给出关闭命令；`docs/review/component-inventory.md:868-880`、`docs/review/accessibility-scope-and-limits.md` 同源过期；台账 F-025/F-029 基于该能力，现亦失效 | 文档：`README.md:158-166`、`README.zh-CN.md:159-167` 改为"不提供屏幕阅读器支持（已移除，见 changelog）"并删掉无效构建命令；`component-inventory.md` §13 标注"已移除"；`perfect-surface-audit` 的 F-025/F-029 由台账维护者在 Task 7 一并改判 | `rg -n "accesskit\|accessibility" README*.md docs/review/component-inventory.md` 结果与 `Cargo.lock` 一致；`cargo build --release -p veil` 仍成功（证明无该 feature 也不影响构建） |
| **P1-2** | 相邻组合矩阵四类边界未覆盖（F-001…F-004） | `tasks.md:11-17` 未勾选；`report-2026-11-27.md:106-117` 只覆盖空行/空格/单内层块 | `crates/veil/tests/syntax_matrix.rs`（ADJB/ADJI/ADJC 生成器） | Task 2.5 判据：`docs/qa/report-*.md` 对应组 NOT RUN = 0，`ADJB` 18×18 全对 |
| **P1-3** | 宽表在窄窗被裁剪，无横向滚动（F-010） | `crates/veil-ui/src/view/table_strategy.rs:8-9` 原文"触底仍超宽时表格向右溢出，由 frame 裁剪（v1 不做横向滚动）" | `view/table_strategy.rs` 列宽策略 + `view/table_view.rs` 绘制/命中；需新增滚动偏移与光标可见性 | 窄窗（版心 < 表最小宽）测试：断言表格可视区域可横向滚动且命中/光标一致 |
| **P1-4** | 物理打印机 NOT RUN（F-012） | `README.zh-CN.md:154`；`w10-output-pipeline-evidence.md:21` 明确"没有改用物理打印机出纸" | 证据项，非代码；按 `deferred-and-manual-register-2026-11-27.md` §1.2 执行 | 实体打印样张（页边距/分页/表格/中文断行）归档 |
| **P1-5** | 系统 DPI 100/125/200 NOT RUN（F-013） | `jobs-lens-report-v2.md:145`、`:270`；仅 150% 有自动首帧检查 | 证据项；§1.1 步骤 | 三档 DPI 下首帧、命中、版心一致性截图 |
| **P1-6** | 真机中文 IME 组合/候选窗 NOT RUN（F-014） | `jobs-lens-report-v2.md:337-341` 已修锚点端口与 preedit 绘制，但六步走查未跑 | 证据项；§1.3 步骤 | 微软拼音/五笔六步走查通过并留证 |

> 说明：台账 F-005…F-009（P1）已由 Task 3（`128e204`）关闭，`tasks.md:19-24` 已勾选，本文件不重复列为 P1；其回归判据保留在台账 §4.1。F-025 与 P1-1 合并（能力已移除，"语音流程未跑"不再是证据缺口而是文档失实）。

### 4.3 新增 P2/P3（登记，不在本轮实施）

| ID | 问题 | 证据 | 级别 |
|---|---|---|---|
| P2-1 | README 中英"1600px 才封顶版心"与实现不符 | `README.md:109`、`README.zh-CN.md:109` vs `geometry.rs:126-142` | P2 |
| P2-2 | README 设置项数量"13（12 可编辑）"与实际 14（13 可编辑）不符 | `README.zh-CN.md:114` vs `settings.rs:206-304` | P3 |
| P2-3 | HTML 导出与屏幕/PDF 使用两套解析器 | `export.rs:143-145/339` vs `veil-core/src/syntax/*`；差分 721 篇（`jobs-lens-report-v2.md:197`） | P2 |
| P2-4 | 16 个产品模块 >1500 行，`pdf_layout.rs` 4196 行 | §1.5 统计 | P2 |
| P2-5 | 端口错误统一 `Result<_, String>`（全仓 227 处） | `app.rs:86-87`、`print.rs:26` | P2 |
| P2-6 | ADR-004"零 `#[cfg]` 在 platform/ 外"未落实 | `ADR-004:21` vs `veil-ui/src/app.rs:14-15/215-226` | P3 |
| P2-7 | ADR-004"macos/linux 未实现即编译失败"未落地，改为兜底 | `platform/mod.rs:130-149` | P3 |
| P2-8 | `App` 18 顶层字段 > ADR-009 目标 ≤12 | `app.rs:895-928` vs `ADR-009:183` | P3 |
| P2-9 | `qa` 测试门面进入产品库公共面 | `veil-ui/src/lib.rs:43`、`veil/src/lib.rs:10-12` | P3 |
| P2-10 | 审计时工作区存在未跟踪临时文件（并行工作产物，非本轮结论；审计期间数量仍在增长） | `git status --porcelain` 实测：`crates/veil-core/tests/zz_scratch_dump.rs`、`crates/veil/tests/zz_scratch_dump.rs`、`crates/veil/tests/zz_scratch_pairs.rs`、`tmp_dump_adj*.txt`/`tmp_dump_br.txt`/`tmp_dump_selected.txt` | P3（发布前须清理；`tests/*.rs` 会被 `cargo test --workspace` 编译，属门禁风险） |

---

## 5. 待验证与证据边界

以下内容本次**没有**取得足以写成结论的新证据，保持"待验证/NOT RUN"，后续不得改写成 PASS：

1. 真机中文 IME 组合、候选窗、上屏（六步走查未执行）。
2. 实体物理打印机出纸；副屏；系统 DPI 100/125/200。
3. 16 种 Mermaid 图型深色观感与 `c4context Rel` 对比度（F-011）。
4. 版本历史面板 5000 快照的阻塞时长（`w11:66`）；5000 标题 `Session::open` 的 p95（`w11:68-70`）。
5. 全量排版 281 ms 对真实用户"大文档滚动/重排"的体感影响（E3 推断边界）。
6. 三项安全边界：实际 peer 核验时机、DNS 切换/rebinding/TLS 证书链、资源上限恶意实测（`README.zh-CN.md:150`）。
7. 本次审计**未运行** `cargo test` / `cargo clippy` / `cargo build`（另一进程正在跑测试，避免锁冲突），§2 中所有数值均为仓库既有实测（E2），不是本次复跑结果。`release-facts-2026-11-27.md` 的 2251/17、0 warning、14,291,456 B 以该文件为准。

---

**一句话结论**：Veil 的架构接缝、错误可见性与状态纪律属于同类产品中的高位水平（P0 = 0，十维 5 好 / 4 可接受 / 1 不足），真正需要立刻处理的是 6 条 P1——其中 3 条是"证据没跑"（IME/物理打印/DPI），1 条是"能力被移除但 README 仍在宣传"（AccessKit），2 条是"复杂内容边界"（相邻矩阵、宽表裁剪）；最大的三个结构性风险是**文档与代码缺少自动校验回路**、**两套解析器并行的长期漂移**、**巨型模块 + stringly 错误对后续迭代速度的侵蚀**。
