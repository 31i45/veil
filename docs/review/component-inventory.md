# Veil 组件使用全景与体积/资源归因审计（v2）

- 审计日期：2026-11-17（v2 决策重判同日）
- 仓库：`d:\VSCode\Veil`，HEAD `ac25a9a`
- 工作树：19 个已跟踪文件修改 + 12 个未跟踪条目（展开为 14 个文件，`git status --short` 共 31 行）；其中 `crates/veil-ui/src/app_tests.rs` 的改动是 Task 2 性能探针的规模数组（非产品逻辑），其余为审计前既有状态
- 产物：`veil.exe` **9,889,792 B**；SHA-256 `BF9C00D287D3C807D92D6ED171BF17F19976B43D7D0A15C30ACC38548C59C0BC`；`.text` 7,228,928 B（73.10%，raw size）
- 工具链：rustc 1.99.1，release（`lto=true`、`codegen-units=1`、`panic=abort`），Windows 11 x64
- 依赖计数：128 个唯一包（口径：`cargo tree -e no-dev -p veil --prefix none` 去重首列）；外部直接依赖 **19 条声明 / 18 个唯一包**（veil 9 + veil-ui 7 + veil-core 3，`base64` 双声明去重）
- 门禁真源：README.md:134 `≤ 10 MiB = 10,485,760 B`（**发布门禁，弱约束**）；`9,900,000 B`（9.9 MB）仅为**内部预警线**。当前余量：对 10 MiB **595,968 B**，对 9.9 MB **10,208 B**
- README 滞后：README.md:134 记 9,784,832 B，实测 9,889,792 B，**滞后 104,960 B**（余量被高估同额）。本报告只标注，不改 README
- 测量工具：cargo-bloat 0.12.1（仅开发机安装，未进仓库）、`dumpbin /headers`、进程级内存采样脚本、独立候选探针
- 改动边界：A-1 的替换（`crates/veil/Cargo.toml` 的 `base64` 声明 + `export.rs` / `mermaid.rs` 两处调用点）已在本轮基线之前落地；除此之外未改 README、产品代码或 `Cargo.toml`。探针工程全部位于工作区外 `%TEMP%\veil-spike\`

## v2 变更记录

1. **门禁真源与弱约束**：明确 10 MiB 是发布门禁、9.9 MB 只是内部预警线；体积从“否决理由”降级为**成本项**，否决必须落在许可证、契约、质量或需求缺失上。
2. **超线性订正**：5 档拟合（R²：M1 0.999604 / M2 0.999991 / M3 0.839097）证明 `index_rebuild` **不是平方增长**，订正为“近似线性 + p50 约 14% 轻微上凸，p95 基本线性”。
3. **内存实测**：四档 × 3 次进程级复测（PrivateMemorySize64 / WorkingSet64），替代 v1 的历史单点引用。
4. **四维重判**：B-1…B-9 改为“候选覆盖度 / 自研具体局限 / 体积性价比 / 维护性价比 / 决策 / 触发条件”矩阵；B-1 由 v1 的“可替换”改判 **Spike-later**，B-2…B-9 维持 Keep 但理由重排。
5. **html2md 许可证**：订正为 **GPL-3.0+**（v1 的许可证记录作废），许可证否决强于体积与 abort 冲突。
6. **fuzzy-matcher 体积**：由单一值改为 **24,576–92,160 B 区间**（旧探针源码已丢失，区间上限不可复现）。
7. **base64 已执行**：A-1 落地，实测 **+1,024 B**（预测 +4,096 B），替换后对 10 MiB 余量 595,968 B。
8. **依赖计数**：外部直接依赖由 17 订正为 **18 个唯一包 / 19 条声明**（`base64` 计入直接依赖）。

---

## 1. 结论摘要

### 1.1 决策表

| 类 | 项 | 决策 | 关键依据 |
|---|---|---|---|
| **A-1** | 手写 base64 → `base64` 0.23 | **Replace now（已执行，+1,024 B）** | 语义覆盖 ~100%，0 新包，正确性/维护性净收益为正 |
| **B-1** | `callouts.rs` → `serde_json` | **Spike-later** | 覆盖 ~90%，但 3 处容错语义回归；当前无用户可见缺陷 |
| **B-2** | `html.rs` → `html2md` | **Keep** | **GPL-3.0+ 许可证否决** + `panic=abort` 不兼容 + 超门禁 1.70 MB |
| **B-3** | `quick_open.rs` 排序 → `fuzzy-matcher` | **Keep** | 质量负收益（`config` 2/10 vs 5/10），体积可承受但不值得 |
| **B-4** | `theme.rs` 加载 → `toml_edit` | **Keep** | 0 边际，但 3 条容错语义相反，未获产品批准 |
| **B-5** | `i18n.rs` → `fluent` | **Keep** | 单语言、无复数/日期需求；+224,256 B、24 包 |
| **B-6** | `search_strategy.rs` → `regex` | **Keep** | 正则不是需求；回溯风险上升；余量 −602,624 B 为成本项 |
| **B-7** | `link_safety.rs` → `url` | **Keep** | 只覆盖 scheme 语法，安全判定反而变弱；46 包 |
| **B-8** | `slug.rs` → `slug` | **Keep** | deunicode 音译会把中文标题改成拼音，改变所有既有锚点身份 |
| **B-9** | `cli.rs` → `clap` | **Keep** | 当前仅 1 个 flag，+293,376 B、16 包属过度 |
| **C** | 其余自研模块（排版链、真值模型、平台缝合等） | **Keep** | 无可用组件；像素契约与文档真值绑定（见 §6.0、§6.3） |

### 1.2 三个数字

- **体积**：9,889,792 B；对 10 MiB 门禁余量 **595,968 B**，对 9.9 MB 预警线余量 **10,208 B**。`.text` 7,228,928 B（73.10%），`.rdata` 2,462,720 B（24.90%）；字体/公式/PDF 字形栈约 **3.13 MB（31.6% exe）**，仍是最大成本块。
- **性能**：击键端到端 p95 **1.358 ms @5,005 块**（同日上一轮，历史引用）；分项合计 p95 从 **1,914.4 µs @2,002 块**升到 **12,758.7 µs @40,005 块**，`index_rebuild` 占比 23.0%→58.1%（5,005 块为 34.3%），但增长阶为近似线性而非平方。
- **内存**：Private 36.12→39.55 MB、WorkingSet 53.44→56.83 MB（10k→200k 词，20 倍词量仅 +3.43/+3.39 MB）；200k 词峰值 56.83 MB，**远低于 150 MB** 预算。

---

## 2. 测量口径与证据等级

| 等级 | 含义 | 本报告用法 |
|---|---|---|
| **实测** | 本次在审计基线上运行命令/探针得到 | 体积基线、PE 节、5 档性能、子段、四档内存、候选边际探针 |
| **静态** | 直接读取源码常量/调用点确认，未在运行期测量 | 线程数、缓存上限、WIC/HTTP 上限、调用路径、行为语义 |
| **历史引用** | 项目既有文档或 v1 的测量，未在本轮复跑 | syntect +7.1 MB、README/ADR 的历史体积数字 |
| **推断** | 基于代码与同类 crate 体量的估计，未实测 | `html2md` 的能力覆盖度 |
| **未确认** | 现有材料无法判定 | 见 §9 与 D.3 清单（`serde_json` 在 Veil 内的真实边际等） |

体积口径：

- 分母统一为 **10 MiB = 10,485,760 B**（发布门禁）。`9,900,000 B` 只用于预警线余量，不作为否决阈值。
- 第三方与自研的字节数来自 `cargo bloat --release -p veil` 的 `.text` 归因；cargo-bloat 以 `optimized + debuginfo` 重建并分析 `target\release\veil.exe`。官方声明数字为估算，受 LTO/内联影响，**允许 ±15% 误差**；模块级数字只作方向性判断。
- 自研模块按 demangle 符号前缀聚合，`veil_ui::app = 447 KB` 包含大量被内联的 `paint`/`render` 泛型单态化，**不能当作“app.rs 代码写得臃肿”的证据**。
- 内嵌资产按源文件字节统计；它们已位于 `.rdata`/`.rsrc` 内，**不与节大小重复相加**。
- 候选替换的边际字节一律取**独立空探针**（107,008 B 基线，release/LTO/cgu=1/abort）实测值，而非 cargo-bloat 归因；两套口径不可混用。
- 当前生产 exe 的 SHA-256 为 `BF9C00D287D3C807D92D6ED171BF17F19976B43D7D0A15C30ACC38548C59C0BC`（9,889,792 B）。v1 记录的两个 SHA-256 属于上一版基线，已作废。

---

## 3. 体积总账

### 3.1 PE 节分解（生产 exe，`dumpbin /headers` 实测）

| 节/项 | raw 字节 | 占 exe | 内容 |
|---|---:|---:|---|
| `.text` | 7,228,928 | 73.10% | 代码（raw size；virtual size 7,228,184） |
| `.rdata` | 2,462,720 | 24.90% | 只读数据、字符串、跳转表、内嵌主题/RGBA |
| `.data` | 2,048 | 0.02% | 可写数据 |
| `.pdata` | 62,464 | 0.63% | 异常展开 |
| `.rsrc` | 91,136 | 0.92% | 图标（89,277 B）+ 版本信息 |
| `.reloc` | 41,472 | 0.42% | 重定位表 |
| PE 头（`SizeOfHeaders`） | 1,024 | 0.01% | 节表与对齐已在各节 raw size 内 |
| **合计** | **9,889,792** | **100%** | |

节变化说明：A-1 执行后 `.text` 的 raw size **未变**（virtual size 7,228,568→7,228,184），`+1,024 B` 全部落在 `.rdata`（2,461,696→2,462,720）。因此 §3.2 的 std/第三方/自研归因仍基于同一份 `.text` 基线。

### 3.2 `.text` 归因

| 归属 | 字节 | 占 exe | 占 `.text` | 说明 |
|---|---:|---:|---:|---|
| std | 1,048,576 | 10.60% | 14.51% | Rust 标准库基线，非项目组件 |
| 第三方 crate | 4,665,532 | 47.18% | 64.54% | 见 §4 |
| 自研（veil-ui + veil + veil-core） | 1,441,280 | 14.57% | 19.94% | 见 §5 |
| 符号未归因 + 节内填充 | 73,540 | 0.74% | 1.02% | 符号合计 7,155,388 vs `.text` raw 7,228,928 |
| **合计** | **7,228,928** | **73.10%** | **100%** | |

> 符号归因基于 `.text` 未变的同一基线（A-1 只改 `.rdata`），允许 ±15% 误差。

### 3.3 内嵌资产

| 资产 | 字节 | 位置 | 用途 |
|---|---:|---|---|
| `icons/veil.ico` | 46,688 | `.rsrc` | 可执行文件/关联图标 |
| `icons/veil-doc.ico` | 42,589 | `.rsrc` | Markdown 文档图标 |
| `crates/veil-ui/assets/themes/nord_light.tmTheme` | 2,217 | `.rdata` | 亮色语法配色 |
| `crates/veil-ui/assets/themes/one_dark.tmTheme` | 2,215 | `.rdata` | 暗色语法配色 |
| `icons/veil-32.rgba` | 4,096 | `.rdata` | 窗口图标原始像素 |
| **合计** | **97,805** | — | 占 exe 0.99% |

字体不在仓库资产中：文本排版用系统字体（`fontdb` 扫描），公式字体随 `latex-rust` crate 编译进二进制。

---

## 4. 已采用的成熟组件台账

### 4.1 外部直接依赖（19 条声明 / 18 个唯一包）

`直接 .text` 指该 crate 自身的符号归因；`占 exe` = 直接 `.text` ÷ 9,889,792 B（保留两位小数；<0.005% 记 `0.00%`）；共享传递依赖单列在“主要传递”列，**不可与直接列简单相加**。口径与 §3.2 的 `.text` 归因一致（cargo-bloat 符号级，允许 ±15% 误差）。

| 依赖 | 版本 | 许可证 | 所属 crate | 直接 `.text` | 占 exe | 主要传递依赖（`.text`） | 运行期频率 | 运行期角色 |
|---|---|---:|---|---:|---:|---|---|---|
| `winit` | 0.30.13 | Apache-2.0 | veil-ui | 100,454 | 1.02% | dpi、raw-window-handle | 每事件/每帧 | 事件循环、窗口、IME |
| `softbuffer` | 0.4.8 | MIT/Apache-2.0 | veil-ui | 4,608 | 0.05% | windows-sys | 每帧 | 软件帧缓冲上屏 |
| `cosmic-text` | 0.19.0 | MIT/Apache-2.0 | veil-ui | 168,550 | 1.70% | harfrust 377,242、swash 45,466、fontdb 10,752、unicode-bidi 17,818 | 每击键/滚动 | 字形 shaping、命中、光栅 |
| `syntect` | 5.3.0 | MIT | veil-ui | 45,466 | 0.46% | fancy-regex 66,048、regex-automata 387,277、regex-syntax 151,859、aho-corasick 134,144 | 按需（代码块） | 语法高亮 |
| `plist` | 1.10.3 | MIT | veil-ui | 22,733 | 0.23% | quick-xml 5,837、base64 3,482 | 启动/切主题 | 解析 `.tmTheme` |
| `toml_edit` | 0.22.27 | MIT/Apache-2.0 | veil-ui | 73,318 | 0.74% | winnow 22,938 | 按需（保存配置） | 保留注释的 TOML 编辑 |
| `memchr` | 2.8.3 | Unlicense/MIT | veil-ui | 18,637 | 0.19% | — | 按需（查找） | 字面量多路扫描 |
| `lapce-xi-rope` | 0.4.0 | Apache-2.0 | veil-core | 18,842 | 0.19% | unicode-segmentation（共享） | 每击键 | Rope 文本 + mark |
| `flate2` | 1.1.10 | MIT/Apache-2.0 | veil-core | 2,560 | 0.03% | miniz_oxide 37,990（与 png 共享） | 按需（Mermaid） | gzip/deflate |
| `arboard` | 3.7.0 | MIT/Apache-2.0 | veil | 207 | 0.00% | image 34,202、clipboard-win 622、moxcms | 按需（剪贴板） | 系统剪贴板 + 位图 |
| `latex-rust` | 3.0.1 | MIT/Apache-2.0 | veil | 771,072 | 7.80% | zeno 208,998、tiny-skia 183,194、skrifa 651,776（共享） | 按需（公式首次渲染） | LaTeX 排版 + 光栅 |
| `png` | 0.18.1 | MIT/Apache-2.0 | veil | 54,579（含传递 0.17.16） | 0.55% | fdeflate 22,733、miniz_oxide（共享） | 按需（粘贴/复制/公式） | PNG 编解码 |
| `pulldown-cmark` | 0.13.5 | MIT | veil | 130,560 | 1.32% | pulldown-cmark-escape 1.6K | 仅导出（HTML） | Markdown 解析为 HTML |
| `notify` | 8.2.1 | CC0-1.0 | veil | 7,885 | 0.08% | notify-types | 启动 + 轮询 | 外部文件变更监听 |
| `pdf-writer` | 0.15.0 | MIT/Apache-2.0 | veil | 20,173 | 0.20% | — | 仅导出（PDF） | PDF 字节写入 |
| `ttf-parser` | 0.25.1 | MIT/Apache-2.0 | veil | 45,670 | 0.46% | — | 仅导出（PDF） | 字体表解析/字形覆盖 |
| `subsetter` | 0.2.6 | MIT/Apache-2.0 | veil | 99,635 | 1.01% | write-fonts 108,134、kurbo | 仅导出（PDF） | 字体子集化 |
| `base64` | 0.23.1 | MIT/Apache-2.0 | veil-core + veil | 3,482 | 0.04% | — | 按需（HTML 导出 / Mermaid） | 标准与 URL-safe base64 编解码（A-1 已执行） |

> **18 个直接依赖自身合计 1,588,431 B = 16.06% exe**（含 `base64` 3,482 B 一次；`plist` 行的“主要传递”列不参与合计）；其余体积为传递依赖、std 与自研（见 §3.2 / §4.2）。

> 排除项：dev-dependencies `proptest`（veil-ui）、`pulldown-cmark`（veil-core 测试）不计入体积台账。

启用 feature（静态，来自三个 `Cargo.toml`）：`syntect` = `default-features=false` + `default-syntaxes` / `regex-fancy` / `plist-load`；`latex-rust` = `features=["png"]`；`pulldown-cmark` = 默认（含 `html`、`getopts`）；`arboard` = 默认（含 `image`/`image-data`）；其余 14 个均为默认 feature。版本与许可证为静态读取（`Cargo.toml` / registry），字节为实测（cargo-bloat），调用点为静态（源码）。

### 4.2 共享技术栈聚合（避免重复计数的视角）

| 技术栈 | 主要 crate | 合计 `.text` | 占 exe | 备注 |
|---|---|---:|---:|---|
| 文本排版 | cosmic-text、harfrust、swash、fontdb、unicode-bidi、unicode-segmentation | 632,526 | 6.40% | 每击键主路径 |
| 字体表解析 | skrifa、read-fonts、font-types、yazi | 1,044,818 | 10.57% | **被 cosmic-text / latex-rust / subsetter 共享**，不可单独归给某一功能 |
| 公式排版与光栅 | latex-rust、zeno、tiny-skia、tiny-skia-path | 1,195,827 | 12.09% | 仅公式渲染使用 |
| PDF 字形子集 | subsetter、write-fonts、ttf-parser | 253,439 | 2.56% | 仅 PDF 导出使用 |
| 语法高亮正则 | syntect、fancy-regex、regex-automata、regex-syntax、aho-corasick | 784,794 | 7.94% | 语法高亮的主要成本在正则引擎，而非 syntect 本体 |
| 窗口与呈现 | winit、softbuffer、raw-window-handle | 108,300 | 1.10% | — |
| 容器/基础 | hashbrown、memchr、smallvec、unicase、bytecount | 128,178 | 1.30% | 全局共享 |

**结论**：字体/公式/PDF 字形栈合计约 **3.13 MB（31.6% exe）**，是最大成本块；其中真正“只为数学公式”的部分约 1.20 MB，其余是与正文排版共享的字体解析。若未来需要显著瘦身，优先级应是“公式字体与共享字体栈的去重/裁剪”，而不是继续在编辑器内核上找体积。

**覆盖门槛**：第三方 crate 按 `.text` 字节降序，≥45,466 B 的 **21 个** crate 累计 **4,250,418 B = 91.1%**（v1 cargo-bloat 基线，本轮 `.text` raw 未变、未复跑；截断点 45,466 B；其中含 §4.2「容器/基础」行里的 `hashbrown` 96,256 B）；其余 crate 合计 415,114 B（8.9%），单个均 <45,466 B。§4.1 的 18 个直接依赖已全量登记（低于截断点的小 crate 仍在 §4.1 单列）；§4.2 的共享栈明细覆盖了主要传递依赖。

---

## 5. 自研模块台账（≥300 行）

行数为源文件总行数（含文件内 `#[cfg(test)]` 测试模块）。§5.1–5.3 的 52 个 ≥300 行模块按总行数口径；§5.4 的 100–299 行模块在括号内标生产行数（`#[cfg(test)]` 起始行 − 1）。测试专用文件（`app_tests.rs` 15,826 行、`chrome/tests.rs` 851 行、`veil-core/tests/*` 共 2,863 行、`veil/examples/*`）不计入台账。`.text` 为符号归因（部分模块因内联显示为“—”，其代码被计入 `app` 或其他符号）。

### 5.1 veil-ui（32 个）

| 模块 | 行数 | `.text` | 运行期角色 | 分类 |
|---|---:|---:|---|---|
| `view/layout.rs` | 6,720 | 161,690 B | 每击键脏块 + 每帧视口排版 | C |
| `interaction.rs` | 4,385 | —（内联进 `app`） | 每击键/每鼠标事件 | C |
| `paint.rs` | 3,885 | 11,878 B | 每帧合成 | C |
| `editor.rs` | 3,306 | 43,418 B | 每次击键 | C |
| `app.rs` | 1,948 | 457,728 B（含泛型单态化） | 每事件/每帧 | C |
| `view/table_strategy.rs` | 1,706 | —（`layout_table_opts` 34.5 KB） | 按需（表格块） | C |
| `view/settings.rs` | 1,678 | — | 按需（设置面板） | C |
| `theme.rs` | 1,595 | 19,354 B | 启动 + 按需 | 主题/调色板 C；**配置加载子组件 B-4** |
| `view/modal.rs` | 1,259 | 6,963 B | 按需（弹窗） | C |
| `document/doc_state.rs` | 1,160 | — | 每帧轮询 + 按需文件操作 | C |
| `html.rs` | 976 | 18,227 B | 按需（富文本粘贴） | **B-2** |
| `document/block_index.rs` | 954 | — | 每击键（索引增量） | C |
| `view/cache.rs` | 848 | 10,240 B | 每帧/每次布局查询 | C |
| `view/table_view.rs` | 829 | — | 按需（表格绘制） | C |
| `view/mermaid.rs` | 740 | 13,210 B | 按需（Mermaid） | C |
| `input.rs` | 743（生产 244） | — | 每次输入事件 | C |
| `i18n.rs` | 739 | — | 每帧读取 | **B-5** |
| `chrome/mod.rs` | 690 | — | 每帧/每事件 | C |
| `print.rs` | 690 | 7,475 B | 仅打印/PDF | C |
| `view/ctx_menu.rs` | 677 | — | 按需（右键） | C |
| `view/search_strategy.rs` | 648 | — | 按需（查找） | **B-6** |
| `view/highlight.rs` | 600 | — | 按需（代码块） | C（syntect 封装） |
| `tokens.rs` | 556 | — | 编译期常量 | C |
| `mode.rs` | 537 | — | 每次击键 | C |
| `view/quick_open.rs` | 466 | — | 按需（浮层） | **B-3** |
| `view/history.rs` | 456 | — | 按需（历史面板） | C |
| `view/search_view.rs` | 442 | — | 每帧（查找打开） | C |
| `view/frame.rs` | 437 | 5,120 B | 每帧 | C |
| `view/style.rs` | 414 | 5,325 B | 每次块布局 | C |
| `chrome/paint.rs` | 372 | — | 每帧 | C |
| `document/snapshot.rs` | 361 | — | 按需（历史动作） | C |
| `view/outline.rs` | 311（生产 184） | — | 按需（目录/大纲面板） | C |

### 5.2 veil 产品层（11 个）

| 模块 | 行数 | `.text` | 运行期角色 | 分类 |
|---|---:|---:|---|---|
| `platform/pdf_layout.rs` | 3,188 | 80,589 B | 仅导出/打印 | C |
| `export.rs` | 3,078 | 92,570 B | 仅导出（HTML） | C；base64 已改用 `base64` 0.23（A-1，已执行） |
| `platform/pdf.rs` | 1,587 | 70,246 B | 仅导出（PDF） | C |
| `platform/print.rs` | 1,231 | 30,003 B | 仅打印/导出 | C |
| `platform/http.rs` | 1,016 | 9,626 B | 按需（远程图片） | C（WinHTTP 系统组件 + 安全校验） |
| `main.rs` | 974 | 45,670 B | 启动 + 按需回调 | C |
| `platform/mod.rs` | 868 | —（`impl$4` 14.5 KB） | 启动 + 按需 | C |
| `platform/assoc.rs` | 640（生产 551） | — | 启动/按需（文件关联注册） | C |
| `platform/ipc.rs` | 496（生产 374） | — | 仅第二实例启动 | C |
| `platform/windows.rs` | 451 | — | 启动/按需（Win32 FFI） | C |
| `platform/wic.rs` | 342（生产 278） | — | 按需（JPEG/系统图像解码） | C |

> 合计：veil-ui 32 + veil 11 + veil-core 9 = **52 个** ≥300 行自研模块。

### 5.3 veil-core（9 个）

| 模块 | 行数 | `.text` | 运行期角色 | 分类 |
|---|---:|---:|---|---|
| `syntax/block.rs` | 3,962 | 合计 `syntax` 138,547 B | 每击键（脏区重解析） | C |
| `syntax/inline.rs` | 3,095 | 同上 | 每击键（受影响块） | C |
| `syntax/mod.rs` | 2,006 | 同上 | 每击键 | C |
| `syntax/model.rs` | 829 | 同上 | 类型定义 | C |
| `session.rs` | 777 | 16,077 B | 每次击键 | C |
| `history.rs` | 425 | — | 每次可撤销编辑 | C |
| `buffer.rs` | 366 | — | 每次击键 | C |
| `mermaid.rs` | 343 | 5,837 B | 按需（Mermaid） | C；base64url 已改用 `base64` 0.23（A-1，已执行） |
| `callouts.rs` | 342 | — | 启动/按需 | **B-1** |

### 5.4 小模块组（100–299 行逐个登记，<100 行按组汇总）

| 模块 | 行数（生产） | 分类 | 说明 |
|---|---:|---|---|
| `veil-ui/src/document/persist.rs` | 275（167） | C | 保存/恢复状态机、自动保存去抖 |
| `veil-ui/src/view/highlight_bg.rs` | 263（123） | C | syntect 后台高亮线程适配层（`bg_loop`，详见 §7.3） |
| `veil-core/src/link_safety.rs` | 247（167） | **B-7** | 安全白名单式 URL 校验；`url` 只覆盖 scheme 语法且安全语义更弱（实测 +221,696 B） |
| `veil-ui/src/anim.rs` | 220（149） | C | 光标闪烁/过渡动画时钟 |
| `veil-ui/src/perf.rs` | 213（161） | C | `VEIL_PERF_PROBE` 帧/击键/RSS 探针 |
| `veil-core/src/toc.rs` | 203（92） | C | 目录提取（基于项目 AST 与块索引） |
| `veil-ui/src/geometry.rs` | 196（125） | C | 矩形/命中纯几何 |
| `veil/src/platform/error.rs` | 175（102） | C | Win32 错误码映射 |
| `veil-ui/src/view/linebreak.rs` | 167（95） | C | 断行规则适配 |
| `veil-ui/src/shell.rs` | 165（165） | C | 窗口/事件循环外壳 + 多实例 IPC 接线 |
| `veil/src/cli.rs` | 162（97） | **B-9** | 手写 CLI 解析；当前仅 1 个 flag，`clap` 实测 +293,376 B |
| `veil-ui/src/document/recovery.rs` | 149（95） | C | 崩溃恢复快照 |
| `veil/src/platform/pdf_media.rs` | 145（123） | C | PDF 图片嵌入 |
| `veil/src/platform/watch.rs` | 143（143） | C | `notify` 薄封装（监听器端口） |
| `veil/src/platform/clock.rs` | 132（101） | C | 可注入时钟 |
| `veil-core/src/slug.rs` | 126（63） | **B-8** | TOC 锚点 slug；`slug` 会音译中文并改变锚点身份，净收益为负 |
| `veil/src/platform/theme.rs` | 125（125） | C | 注册表系统主题读取 |
| `veil-ui/src/image.rs` | 98（81） | C | 平台图像加载端口；PNG 走 `png`，JPEG 走系统 WIC |
| 其余 <100 行文件 | — | C | 几何/路径/常量等纯逻辑，无组件可替 |

> `crates/veil/build.rs`（145 行）是构建脚本，不参与运行期与二进制代码，不计入台账。

---

## 6. 三分类判定

### 6.0 像素排版契约备忘

**边界结论**：像素契约只背书“文档字节 → 块 → 几何 → 字形 → 位图 → 命中/光标”这条链；**不背书** JSON 解析、HTML→Markdown、CLI 解析、slug、配置文本解析与文案表。这些邻近模块必须按四维（覆盖度 / 自研局限 / 体积性价比 / 维护性价比）单独判断，详见 §6.2。

#### 6.0.1 不变量 I1–I9

| 编号 | 不变量（摘要） | 代码锚点 | 违反后的可观测后果 |
|---|---|---|---|
| **I1 字节↔块** | 字形携带文档字节 `(start,end)`，块有 `Range`，非激活标记段由 `OffsetMap` 双向换算 | `view/layout.rs:1-7, 509-544, 264-358`；`document/block_index.rs:19-48` | 命中/选区/复制/导出落点整体错位 |
| **I2 脏块增量** | 只重排脏块；单块原位编辑替换该块并平移其后 top；缓存按 `BaseKey`+激活指纹复用 | `block_index.rs:107-132, :136-`；`view/cache.rs:1-17, 239-266` | 每击键退化为全篇重排，或残留旧块高导致视口跳变 |
| **I3 离屏块不重排** | 视口外只用 `est_h` 估算；仅 `scroll ± 1 屏` 进入排版；`geometry_stale` 控制整篇重算 | `block_index.rs:29-31, 428-431`；`paint.rs:854-887`；`app.rs:1854-1866` | 大文档成本随总块数上涨（当前 layout/raster p95 几乎不随 N 变） |
| **I4 表格像素网格** | 列宽由字形实测后按比例收缩（`MIN_COL_WIDTH`），非字符格；每格有 `doc_start` 与活动格 `caret_map`；网格线 1px | `view/table_strategy.rs:1-17, 33, 108-110, 126-165, 521-544, 699-718`；`table_view.rs:259, 613, 709-721` | 单元格点击落错偏移；公式/图片占位与网格错位；高 DPI 糊线 |
| **I5 精确命中/光标** | 像素→字节经 `Buffer::hit` + `layout_to_doc`；字节→矩形经 `doc_to_layout` 逐 run 插值；表格走 `TableLayout::hit` | `layout.rs:1657-1665, 1667-1708`；`interaction.rs:741-744`；`table_strategy.rs:167-268` | 点击偏一个字符；光标贴错折行；表格与正文落点口径分叉 |
| **I6 主题/缩放即时重排** | `BaseKey` 含 `width_q/scale_q/theme_rev/session_rev/image_*/font_size_q/line_height_q/font_family`，任一变化使缓存自然失配 | `view/cache.rs:83-125, 425-432`；`prune_viewport` | 换主题/缩放后残留旧色或旧字宽 |
| **I7 光栅位图与合成** | 光栅输出 RGBA8（straight alpha），按块缓存；`Frame::blit` 裁剪；面积上限 24,000,000 px；打印/导出复用 | `layout.rs:86, 1757-1774`；`view/frame.rs:50-103, 104-128` | 图片/公式溢出到相邻块；极端字号 OOM；打印与屏幕不一致 |
| **I8 装饰/选区同源** | 选区、删除线、下划线、mark 底色、Sub/Sup 基线偏移全部由同一 `OffsetMap` + run 几何派生 | `layout.rs:518-528, 1719-1755, 1766-1774` | 高亮与字形错位；上下标处选区高度错误 |
| **I9 消隐/Reveal** | 标记段按 caret 激活位消隐/显示；激活指纹参与缓存键，块平移后不误命 | `layout.rs:3-7, 560-600`；`cache.rs:239-266` | 标记残留“已显示”或被吞 |

#### 6.0.2 契约背书的 13 个模块

| 模块 | 承担不变量 | 换成通用组件会破坏什么 |
|---|---|---|
| `view/layout.rs`（6,720 行） | I1 I2 I5 I6 I7 I8 I9 | `cosmic-text` 只提供 shaping/行布局；块间距、标记消隐、`OffsetMap`、表格同源光栅是我方可观测契约 |
| `paint.rs`（3,885 行） | I2 I3 I5 I7 | “整棵树重绘”的 UI 框架会取消离屏不重排（I3）与逐块光栅缓存（I7） |
| `view/frame.rs`（437 行） | I7 | `blit/blit_scaled/fill_rect` 是 softbuffer 像素原语；egui/wgpu 会接管 surface 与裁剪，打印 GDI 路径分叉 |
| `view/cache.rs`（848 行） | I2 I6 I7 | 正确性由因果键（`cache.rs:1-17`）承载；通用缓存没有“字节 range + 激活指纹 + 主题代次”的键等式 |
| `view/style.rs`（414 行） | I1 I5 I8 | 样式段按字节区间产出；CSS 式级联没有文档字节坐标，需二次映射 |
| `view/table_strategy.rs`（1,706 行） | I4 | 列宽是像素实测、`doc_start` 是字节偏移、`caret_map` 只在活动格成立；`comfy-table` 输出字符网格 |
| `view/table_view.rs`（829 行） | I4 I7 | `glyph_anchor`（`:259`）把占位字形换成公式/图片锚点；网格线是 1px 像素 |
| `geometry.rs`（196 行） | I5 | `Rect::contains` 与窗口夹取是命中/浮层唯一坐标系；UI 框架会引入第二套布局坐标 |
| `interaction.rs`（4,385 行） | I5 | 像素→文档偏移、表格 `hit`、浮层优先级；widget 树会抢走事件与坐标 |
| `editor.rs`（3,306 行） | I1 I5 | caret 是文档字节偏移，IME preedit 是视图态（`editor.rs:1, 72, 254-294`）；通用文本控件把 caret 变成内部索引 |
| `mode.rs`（537 行） | I5 | 按键路由依赖浮层/焦点状态（`mode.rs:83`），与几何/命中同源 |
| `document/block_index.rs`（954 行） | I1 I2 I3 | 唯一允许全量枚举块的位置（`:1-3, 101-118`）；`est_h`/`top` 是 I3 的基础，通用排版树没有“估算 + 平移”模型 |
| `app.rs`（1,948 行） | I2 I6 | 脏帧编排、`Revs`、`geom_dirty`、周期唤醒（`app.rs:44-50, 1854-1866`）是缓存键的代次来源 |

#### 6.0.3 契约**不能**背书的邻近模块

| 模块 | 位置 | 与像素链的关系 | 处理 |
|---|---|---|---|
| `callouts.rs` | `crates/veil-core/src/callouts.rs` | 启动时 JSON → `CalloutRegistry`，只影响块分类与配色，不产几何 | B-1 独立判 |
| `html.rs` | `crates/veil-ui/src/html.rs` | 粘贴输入转换，输出 Markdown 文本，不进排版链 | B-2 独立判 |
| `quick_open.rs` | `crates/veil-ui/src/view/quick_open.rs` | `score/filter`（`:171-223`）是纯排序；同文件 `layout/hit`（`:236-283`）只是浮层矩形 | B-3 只判排序 |
| 配置加载 | `theme.rs:543-796` | 文本 → `Config`；`to_toml_preserving` 已用 `toml_edit` 写回 | B-4 独立判 |
| `i18n.rs` | `crates/veil-ui/src/i18n.rs` | 静态 `Key`↔字符串表；`t/tf` 只拼字符串（`:596-617`） | B-5 独立判 |
| `search_strategy.rs` | `crates/veil-ui/src/view/search_strategy.rs` | `find_all`（`:282-299`）返回字节 `Range`，供高亮/跳转；不含几何 | B-6 独立判 |
| `link_safety.rs` | `crates/veil-core/src/link_safety.rs` | 安全白名单判定；只决定是否写入 href | B-7 独立判 |
| `slug.rs` | `crates/veil-core/src/slug.rs` | 标题文本 → 锚点 id | B-8 独立判 |
| `cli.rs` | `crates/veil/src/cli.rs` | 启动参数 → `Launch` | B-9 独立判 |

> `quick_open.rs`/`search_strategy.rs` 虽 import `Rect` 与 `tokens`，但只**消费**几何常量，不承担 I1–I9 中任何一条；替换其排序/扫描算法不会改写像素链。反之，替换 `table_strategy.rs` 必然改写 I4。

#### 6.0.4 候选为何过不了契约

| 候选 | 覆盖范围 | 具体冲突 |
|---|---|---|
| `cosmic-text` | 仅 shaping / 行布局 / `hit` / 光栅字形 | 已被当作**底层**使用（`layout.rs:11-14`）；没有块 `Range`、标记消隐（I9）、脏块增量（I2）、离屏估算（I3）、表格网格（I4）。当“替换”等于删掉 I1–I4 的实现者 |
| `comfy-table` | 列宽/折行/边框 → `String` | 列宽是 `unicode-width` 字符格而非 DPI/字号下的字形实测；无 `doc_start`、无 `caret_map`；边框是 `+---+` 字符而非 1px 像素。桥接需“像素宽→字符格→反推像素”的双份计算 |
| `egui` / `wgpu` | 整窗 UI / GPU 合成 | 接管 surface 与事件循环，I7 的 `Frame::blit` 与打印 GDI 路径被替换；I3 的“只排可见块”变成框架内部裁剪，不可控；I5 的命中由 widget 树决定（ADR-005、ADR-011 已否决） |
| `taffy` | flexbox/grid 布局树（无文本 shaping） | 输入是样式节点而非“字节 range + 块 kind”；输出无字形/字节锚点、无脏块增量入口。接回 I1/I5 必须自建 offset↔node 映射，等于再写一层 `OffsetMap` |
| `typst-math` | 仅数学排版 | 只覆盖公式**内容**，不覆盖块高、占位宽、`PlacedImg` 基线、缓存键；且实测 +15,857,664 B、267 个新 crate，超门禁（ADR-002 2026-11-16 复核） |

### 6.1 A：应当用成熟组件但自研了（1 项，已执行）

#### A-1 手写 base64 编码 → `base64` 0.23.1（已执行）

- 位置：`crates/veil/src/export.rs::base64_encode`（HTML 导出 data URI，`STANDARD`）、`crates/veil-core/src/mermaid.rs::base64url_encode`（mermaid.live URL，`URL_SAFE_NO_PAD`）。
- 原状：两处手写编码表与填充/padding 分支，约 44 行；padding 与 url-safe 细节靠人工保证。
- 候选：`base64` 0.23.1，MIT/Apache-2.0，**已在二进制中**（`plist` 解码路径 + 本轮新增两处直接调用），直接 `.text` 3,482 B，新增包 0。
- 验证结果（实测）：预期边际 +4,096 B（v1 探针：空 107,008 → 仅解码 108,544 → 编码+解码 112,640；编码路径相对仅解码基线 +4,096），**实际整包只增 +1,024 B**（9,888,768 → 9,889,792），且 `.text` raw 不变、增量全部落在 `.rdata`。替换后对 10 MiB 余量 **595,968 B**，对 9.9 MB 预警线余量 10,208 B。
- 语义覆盖：`STANDARD`（带 `=` 填充）与 `URL_SAFE_NO_PAD`（RFC 4648 §5，无 padding）分别对应两处原实现；编码结果由既有向量测试固定（`export.rs:1877-1884`、`mermaid.rs:250-255`）。
- 判定：**A，已执行**。删两套手写编码表、保留薄封装，正确性与维护性净收益为正；体积成本 +1,024 B（0.0098% 门禁）可忽略。

### 6.2 B：自研有合理性，但可评估替换（9 项，四维矩阵）

> 体积口径：分母 10 MiB；边际取独立空探针（107,008 B 基线）。**体积只作成本项**：v1 的“体积一票否决”已废除（当时的 11,232 B 余量是历史基线，本轮重冻后余量为 595,968 B）。否决理由必须落在许可证、契约、质量或需求缺失上。

| 编号 | 组件 → 候选 | 边际 | 占门禁 | 替换后余量 | 新增包 / 许可证 | 决策 |
|---|---|---:|---:|---:|---|---|
| B-1 | `callouts.rs` → `serde_json` 1.0.154 | +61,952 | 0.59% | 534,016 | 0（已在树，经 syntect）/ MIT OR Apache-2.0 | **Spike-later** |
| B-2 | `html.rs` → `html2md` 0.2.17 | +2,291,712 | 21.86% | **−1,695,744** | 43 / **GPL-3.0+** | **Keep** |
| B-3 | `quick_open.rs` 排序 → `fuzzy-matcher` 0.3.7 | +24,576（旧探针 92,160，源码丢失） | 0.23%（区间 0.23–0.88%） | 571,392（区间 503,808） | 2–3 / MIT | **Keep** |
| B-4 | `theme.rs` 加载 → `toml_edit` 0.22.27 | 0（已在树） | 0% | 595,968 | 0 / MIT OR Apache-2.0 | **Keep** |
| B-5 | `i18n.rs` → `fluent-bundle` 0.16 | +224,256 | 2.14% | 371,712 | 24 / Apache-2.0 OR MIT | **Keep** |
| B-6 | `search_strategy.rs` → `regex` 1.13 | +1,198,592 | 11.43% | **−602,624** | 0（已在树未链接）/ MIT OR Apache-2.0 | **Keep** |
| B-7 | `link_safety.rs` → `url` 2.5.8 | +221,696 | 2.11% | 374,272 | 46（全平台口径）/ MIT OR Apache-2.0 | **Keep** |
| B-8 | `slug.rs` → `slug` 0.1.6 | +6,144 | 0.06% | 589,824 | 2 / MIT OR Apache-2.0 | **Keep** |
| B-9 | `cli.rs` → `clap` 4.6.8 | +293,376 | 2.80% | 302,592 | 16 / MIT OR Apache-2.0 | **Keep** |

#### B-1 `serde_json`（Spike-later）

- **候选覆盖度**：约 90%——JSON 语法 100%，但 3 处容错语义相反（见下）。
- **自研具体局限**：`callouts.rs:94-102` 的 `\u` 逐 4 位 `char::from_u32`，不处理代理对，合法 emoji/非 BMP 字符使**整个文件**判非法（`from_json` → `unwrap_or_default()`，`:18-21`）；`:87-104` 只处理 `" \ / n t r u`，缺 `\b`/`\f`；`:156-159` 数组尾逗号整文件拒绝，而 `:199-206` 对象尾逗号反而接受（口径不对称）；`:106` 原样 `push(c)`，不校验字符串内控制字符；`:191-198` + `:112-147` 非字符串值静默跳过；`:217-224` 重复键后者生效但无诊断；`:149-208` 全 `Option` 链，错误不可定位。
- **体积性价比**：+61,952 B = 0.59%，余量 534,016 B，0 新包；但 `serde_json` 已由 `syntect` 引入，Veil 内真实链接边际**未确认**（≤61,952 B）。
- **维护性价比**：删约 172 行解析器、保留 `validate`；换来完整 JSON 语义与可定位错误，代价是 3 处“宽容→严格”回归：对象尾逗号当前接受、字符串内控制字符当前接受、非法值当前逐键忽略。
- **决策**：**Spike-later**（当前无用户可见缺陷）。
- **触发条件**：① callouts 需要嵌套/数组/非字符串值；② 收到 emoji/转义导致的配置失效报告；③ 批准严格化（或接受“非法即整表为空”并保留手写回退）。

#### B-2 `html2md`（Keep，许可证否决）

- **候选覆盖度**：约 60–70%（推断）：不做 scheme 白名单；围栏语言、未知实体、GFM 表格策略不同。
- **自研具体局限**：`html.rs:592, 608` 的 `link_url_is_safe` 是粘贴入口安全闸门；`render_pre`（`:493-499`）输出围栏语言；`:740-769` 未知实体原样保留；`:434-491` 表格只做最宽行补齐。替换会同时丢失安全行为与保真。
- **体积性价比**：+2,291,712 B = 21.86%，超门禁 1,695,744 B；43 个传递包。
- **维护性价比**：**GPL-3.0+ 许可证否决**（`cargo metadata` 口径；v1 的许可证记录作废），接入即改变整体分发许可义务；`panic=abort` 下构建失败（原文：`the linked panic runtime panic_unwind is not compiled with this crate's panic strategy abort`）。
- **决策**：**Keep**。
- **触发条件**：出现“小体积 + 支持 abort + 许可证兼容 + 可注入 scheme 过滤”的 HTML→MD 组件（当前不存在）。

#### B-3 `fuzzy-matcher`（Keep）

- **候选覆盖度**：<50%：无最近文件加成、无文件名优先，长度惩罚口径不同；243 候选下 `config` precision@10 **2/10 vs Veil 5/10**。
- **自研具体局限**：`quick_open.rs:171-202` 的 `score()` 是**贪心首个匹配**而非最优子序列 DP；`:172-176` 每候选每次击键两次分配（`to_lowercase()` + `Vec<char>`）；`:205-211` 无 frecency/路径深度权重，最近文件只在同分时 +10。
- **体积性价比**：本轮 +24,576 B（3 包）= 0.23%，余量 571,392 B；旧探针 92,160 B 源码已丢失，故记 **24,576–92,160 B 区间**（0.23–0.88% 门禁），区间上限不可复现。
- **维护性价比**：皆可承受，但质量是负收益；ADR-014 的结论是“在手写内演进”。
- **决策**：**Keep**。
- **触发条件**：排序质量成为实际痛点，且证明通用引擎在 `config` 类查询不劣于手写；届时优先在手写内引入 DP（零体积）。

#### B-4 `toml_edit` 加载路径（Keep）

- **候选覆盖度**：约 80%——TOML 语法 100%，容错语义 3 条相反。
- **自研具体局限**：`theme.rs:639-744` 手写行解析与 `:543-605` 的 `toml_edit` 写回构成双解析器；`:639-796` 的 `parse_config`/`strip_comment`/`unquote`/`parse_bool`/`parse_window_size` 约 130 行。三条语义差异：非法值（`mode = `）当前只忽略该键，`toml_edit` 整文档失败；重复键当前后者生效，`toml_edit` 整文档失败；历史 `[theme]` 段当前按顶层生效，`toml_edit` 解析为 `theme.mode`。
- **体积性价比**：0 边际（已在树，写路径已链接），0 新包，余量不变。
- **维护性价比**：统一 AST 可删约 130 行并消除“保存后再加载”的漂移，但用户旧配置可能出现“配置突然失效”。
- **决策**：**Keep**（维持 S2 No-Go）。
- **触发条件**：配置 schema 需要嵌套表/数组；或产品明确批准上述 3 条语义变更并给出迁移提示。

#### B-5 `fluent`（Keep）

- **候选覆盖度**：约 100% 的字符串替换，但当前无复数/性别/日期需求。
- **自研具体局限**：`i18n.rs:13-37` 的 `keys!` 宏同时生成 `Key` 与 zh-CN 表（穷举性由构造保证）；`:548` `EN_US` 为空表；`:563` `CURRENT_LOCALE` 固定 `"zh-CN"`；`:602-617` `tf` 只做 `{}` 顺序替换，参数不足时静默空串。无复数规则、无日期/数字格式、无运行时切语言、无消息引用。
- **体积性价比**：+224,256 B = 2.14%，24 包，余量 371,712 B。
- **维护性价比**：引入运行时解析与回退链，收益为零（单语言 + 纯插值）。
- **决策**：**Keep**。
- **触发条件**：需要运行时切换语言并支持复数/性别/日期本地化（届时评估 `fluent` 或更小的 `icu4x` 子集）。

#### B-6 `regex`（Keep）

- **候选覆盖度**：字面行为 100%；新增的正则能力当前**无需求**。
- **自研具体局限**：`search_strategy.rs:282-299` 用 `memchr::memmem::find_iter` 做非重叠字面扫描；`:291-295` 大小写折叠失败时**静默回退为大小写敏感**；`fold_case`（`:258-272`）对 KELVIN SIGN 等 Unicode 折叠返回 `None` 即回退。不支持正则/通配/词边界。
- **体积性价比**：+1,198,592 B = 11.43%（**v1 历史探针，本轮未复跑**），替换后余量 **−602,624 B**（成本项，非首要否决理由）。
- **维护性价比**：搜索语义与病态输入的回溯风险上升，收益仅“新增正则能力”。
- **决策**：**Keep**。
- **触发条件**：产品把正则搜索列为一等功能并重新立项体积预算（README 红榜已记录）。

#### B-7 `url`（Keep）

- **候选覆盖度**：约 30%——`url` 只做解析，不做实体/百分号混淆还原，也无白名单语义。
- **自研具体局限**：`link_safety.rs:12` 白名单 `http/https/mailto/tel`；`:73-84` 判定前做最多 4 轮实体/百分号解码；测试 `:194-219` 覆盖 `javascript&colon;`、`%6a%61vascript:`、`java\tscript:`。这些安全判定是 `Url::parse` 不提供的。
- **体积性价比**：+221,696 B = 2.11%，46 包（全平台口径），余量 374,272 B。
- **维护性价比**：替换后仍须保留 `decode_obfuscation`，收益只剩约 20 行 scheme 语法解析；**安全判定反而变弱**。
- **决策**：**Keep**。
- **触发条件**：需要完整 URL 规范化、IDN/punycode 校验或相对路径解析（当前 `percent_decode_path` 已覆盖本地图片）。

#### B-8 `slug`（Keep）

- **候选覆盖度**：约 70%——无 NFC 规范化；`slug` 走 deunicode 音译，我方保留 CJK。
- **自研具体局限**：`slug.rs:16-62` 实现 ASCII 小写、CJK 原样保留、空白/`-` 合并、标点丢弃、首尾 `-` 去除、空 slug 参与去重、冲突跳到下一个可用（测试 `:117-125`）；缺口是无 NFC/NFD、非 ASCII 不做大小写折叠、无最大长度。
- **体积性价比**：+6,144 B = 0.06%，2 包，余量 589,824 B（体积几乎免费）。
- **维护性价比**：`中文`→`zhong-wen` 会改变**所有既有标题锚点**，破坏文档内链接、外部书签与已导出 HTML；体积便宜但净收益为负。
- **决策**：**Keep**。
- **触发条件**：需要音译或跨平台 slug 规范化，且接受锚点迁移（须提供保留旧锚点的兼容层）。

#### B-9 `clap`（Keep）

- **候选覆盖度**：约 80%——`--` 终止、单位置参数、中文错误文案可覆盖，但当前无 `--help/-V` 需求。
- **自研具体局限**：`cli.rs:22-53` 仅支持 `--new-window`、`--`、单位置参数；`:55-96` 的 `absolute_path`/`normalize_lexically` 与 CLI 框架无关，替换后仍保留；错误文案硬编码中文（`:10, 19, 59, 62, 68, 75, 89`），未走 `i18n`。
- **体积性价比**：+293,376 B = 2.80%，16 包，余量 302,592 B。
- **维护性价比**：对 1 个 flag 的 CLI 属过度；帮助文案/退出码还会引入新行为面。
- **决策**：**Keep**。
- **触发条件**：CLI 选项 >10，或需要 shell completion/帮助本地化。

### 6.3 C：自研完全正确（无可用组件）

| 组件组 | 代表模块 | 为什么没有可用组件（具体契约冲突） |
|---|---|---|
| 文档真值与增量解析 | `veil-core/syntax/*`、`session.rs`、`buffer.rs`、`history.rs` | `pulldown-cmark`/`comrak` 只提供一次性事件流：① 无脏块增量重解析；② 不保留每块字节 range 与 grapheme 偏移；③ AST 不含 callouts/表格模型/公式节点；④ 撤销需要 rope mark 重映射与事务分组，通用 undo crate 只提供栈。替换等于重写真值模型。 |
| 块级排版与像素绘制 | `view/layout.rs`、`paint.rs`、`frame.rs`、`view/cache.rs`、`view/style.rs`、`view/table_*`、`chrome/*` | 受 I1–I9 契约背书（§6.0.2）。`cosmic-text` 只做 shaping/行布局；`comfy-table` 输出字符网格；`egui`/`wgpu` 接管 surface 与事件循环（ADR-005/011 否决）；`taffy` 无字节锚点与脏块增量。 |
| 编辑器交互与 IME | `interaction.rs`、`editor.rs`、`mode.rs` | 按键→意图→文档事务的状态机、preedit 边界、选区/剪贴板/菜单路由，没有可嵌入的 Rust 组件覆盖；GUI 框架会消灭自绘 chrome 优势。 |
| 平台缝合 | `platform/http.rs`、`platform/print.rs`、`platform/mod.rs`、`main.rs` | 已使用系统成熟组件（WinHTTP、GDI、WIC、Win32 剪贴板），自研部分只是 FFI 与端口注入。`ureq`/`reqwest` 等会引入 TLS 栈与依赖膨胀；WinHTTP/GDI 本身就是系统成熟组件，本项目未重复实现 HTTP/打印协议，因此判 C 而非 B。 |
| 文档生命周期与面板 | `doc_state.rs`、`persist.rs`、`recovery.rs`、`block_index.rs`、`snapshot.rs`、`settings.rs`、`modal.rs`、`ctx_menu.rs`、`search_view.rs`、`view/history.rs` | 打开/保存/冲突备份/自动保存/外部轮询是与文档真值绑定的事务状态机，没有通用组件；`rfd`（文件对话框）与 egui 已被 ADR-005/011 否决（会引入独立窗口/渲染栈并破坏自绘 chrome），其余自绘面板需要项目字阶令牌与命中模型。 |
| 导出与打印 | `export.rs`、`print.rs`、`pdf_layout.rs`、`pdf.rs` | 解析已用 `pulldown-cmark`、PDF 字节已用 `pdf-writer`、字形已用 `ttf-parser`/`subsetter`、base64 已用 `base64`；剩余是项目特定保真（公式 SVG、Mermaid、资源内联、分页/目录）。 |
| 语法高亮封装 | `view/highlight.rs`、`view/highlight_bg.rs` | 引擎已换成 `syntect`；自研部分只是增量行缓存（64 块/2,048 行）、语义段压缩与后台 `bg_loop` 线程（第 55 行 `thread::spawn`），属于适配层。 |

---

## 7. 运行期贡献与资源画像

### 7.1 第三方组件的运行期角色

下表的 CPU/内存特征来自源码调用点（静态确认）与 §7.2/§7.3 的实测；未单独标注内存数字的行不构成实测内存数据。

| 组件 | 触发频率 | 关键调用点 | CPU/内存特征 |
|---|---|---|---|
| `winit` | 每个输入/窗口事件 | `app.rs` `ApplicationHandler` | 主线程事件循环；常驻开销低 |
| `softbuffer` | 每帧 | `app.rs` `Surface::new` + `paint` | 一次整帧拷贝；显存/内存由窗口尺寸决定 |
| `cosmic-text` | 每击键（脏块）、滚动（新可见块） | `view/layout.rs` `LayoutEngine` | shaping + 光栅缓存；启动时 `FontSystem` 扫描系统字体（CPU/内存未单独实测） |
| `syntect` | 按需（代码块首次/编辑；后台线程） | `view/highlight.rs`、`view/highlight_bg.rs` | 加载后常驻 **+7.1 MB**（历史实测）；1000 行 Rust 后 +22.2 MB；`fancy-regex` 回溯在病态输入上可能放大 |
| `latex-rust` | 按需（公式首次渲染/缓存失效） | `main.rs` `LatexMath` | 单次排版 + PNG 光栅；结果进缓存；字体栈共享 |
| PDF 栈（`pdf-writer`/`ttf-parser`/`subsetter`） | 仅导出 | `platform/pdf.rs` | 瞬时内存随页数与字体子集增大；导出结束释放 |
| `pulldown-cmark` | 仅 HTML 导出 | `export.rs` | 线性内存；不参与编辑路径 |
| `arboard` | 按需（复制/粘贴） | `main.rs` `Sys` | 位图剪贴板触发 `image` 解码，瞬时内存随图片尺寸 |
| `png` | 按需（粘贴/复制/公式） | `main.rs` `decode_png_rgba` | 解码缓冲受图片像素上限约束 |
| `notify` | 启动（事件驱动）+ 应用层每 300 ms 取事件 | `platform/watch.rs` + `document/doc_state.rs:388` | 1 个后台线程；CPU 极低 |
| `plist` | 启动/切主题 | `theme.rs` | 解析 2 个 ≤2.2 KB 文件；一次性 |
| `toml_edit` | 按需（保存配置） | `theme.rs` `edit_existing_toml` | 极小；仅保存路径 |
| `memchr` | 查找打开期间每次查询变化 | `search_strategy.rs` | 线性扫描，无额外内存 |
| `lapce-xi-rope` | 每击键 | `veil-core/buffer.rs` | 内存 ≈ 文档大小 + 树/块开销 |
| `flate2` | 按需（Mermaid URL） | `veil-core/mermaid.rs` | 极小缓冲 |
| `base64` | 仅导出（data URI）、按需（Mermaid URL） | `export.rs`、`veil-core/mermaid.rs` | 极小缓冲；A-1 后无手写表 |

### 7.2 实测性能（5 档 × 3 次，2026-11-17，release，`--test-threads=1`）

规模数组：`[286, 715, 1429, 2857, 5715]`（`stress_doc_n`，对应 2,002 / 5,005 / 10,003 / 19,999 / 40,005 块）。每档取 3 次运行的逐指标中位数，单位 µs。

| 块数 | core p50 | core p95 | index p50 | index p95 | layout p95 | raster p95 | compose p95 | total p95 | index 占比 % | index ns/块 (p50) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 2,002 | 121.70 | 177.50 | 300.10 | 441.10 | 121.30 | 342.40 | 822.50 | 1,914.40 | 23.0 | 149.9 |
| 5,005 | 288.00 | 372.20 | 810.00 | 941.50 | 130.80 | 331.60 | 978.30 | 2,743.90 | 34.3 | 161.8 |
| 10,003 | 633.10 | 739.80 | 1,651.70 | 1,804.20 | 116.50 | 204.60 | 1,144.20 | 3,979.30 | 45.3 | 165.1 |
| 19,999 | 1,092.20 | 1,276.00 | 3,354.60 | 3,679.30 | 127.70 | 210.90 | 1,617.90 | 6,929.10 | 53.1 | 167.7 |
| 40,005 | 2,250.20 | 2,621.20 | 6,861.60 | 7,411.40 | 149.10 | 221.10 | 2,355.90 | 12,758.70 | 58.1 | 171.5 |

> `index 占比` = 同一档 `index p95` 中位 ÷ `total p95` 中位；各档 3 次占比的中位数与表值差 ≤0.3 个百分点。

`index_rebuild` 子段中位数（`perf_index_subsegments`；`index_sum` 为四段之和，生产 `refresh_index` 另含 `sync_expanded_details`，探针未计）：

| 块数 | blocks_enum p50 | geom_fill p50 | recompute p50 | stats p50 | index_sum p50 | index_sum p95 |
|---:|---:|---:|---:|---:|---:|---:|
| 2,002 | 30.9 | 83.8 | 52.7 | 0.00 | 168.6 | 237.1 |
| 5,005 | 78.4 | 512.0 | 148.2 | 0.00 | 755.8 | 906.8 |
| 10,003 | 366.1 | 958.5 | 319.8 | 0.00 | 1,646.5 | 1,910.3 |
| 19,999 | 691.8 | 1,928.9 | 647.9 | 0.00 | 3,269.1 | 3,692.7 |
| 40,005 | 1,453.0 | 3,970.9 | 1,324.8 | 0.00 | 6,753.1 | 7,540.7 |

子段占比（按中位 p50 计算）：2,002 块 = `geom_fill` 49.7% / `blocks_enum` 18.3% / `recompute` 31.3%；5,005 块 = 67.7 / 10.4 / 19.6%（各轮 `geom_fill` 67.6–70.0%）；10,003–40,005 块稳定在 **`geom_fill` 58–59%、`blocks_enum` 21–22%、`recompute` 19–20%、`stats` ≈ 0**。各子段中位独立取值、分母取 `index_sum` 中位，因此低档占比之和为 97.7–99.3%，不一定恰为 100%。

增长阶拟合（index p50 中位数）：

| 模型 | 参数 | R² | 调整 R² | 判定 |
|---|---|---:|---:|---|
| M1：y = a·N | a = 0.17035337 µs/块 | 0.999604 | 0.999472 | 残差符号系统性“先负后正”，有轻微上凸 |
| M2：y = a·N·log2N + b·N | a = 0.00352758；b = 0.117571 | 0.999991 | 0.999982 | 样本内最优，但 5 点 2 参数有过拟合风险 |
| M3：y = a·N² | a = 4.57914e-6 | 0.839097 | 0.785460 | **明确排除**：低 N 档低估 45%–94% |

解读：

- **增长阶**：`index_rebuild` **近似线性 + p50 约 14% 轻微上凸**（每块 p50 149.9→171.5 ns，其中 2k→5k 一次性 +7.9%，之后每档仅 +1.6%~+2.3%）；**p95 基本线性**（每块 220.3→185.3 ns 持平/微降，倍率 1.92–2.13×/倍）。**不是平方**。v1 的“唯一随规模超线性增长”断言已订正：轻微上凸可用 N log2N 描述，也可能部分是低 N 档工作集更小、缓存命中更好的效应，现有 5 点无法区分。
- **index 占比**从 23.0% 升到 58.1%，原因是 index 与 core 随 N 增长，而 layout/raster 只与可见块相关、compose 弱于线性；占比上升本身不代表增长阶恶化，只说明大文档下 index 是主要成本来源。
- **5,005 块端到端**（`perf_keystroke_path`，同日上一轮，300 次）：p50 **0.963 ms**、p95 **1.358 ms**、max **1.625 ms**，远低于 16.7 ms 帧预算。“分项合计”是各段独立测量相加，不等于端到端路径（端到端有缓存命中与重叠），两者不可直接对比；40,005 块分项 p95 12.76 ms 仍有余量。
- **冷启动**（同日上一轮，5,005 块）：`perf_cold_first_edit` app 137.581 ms + 首编辑 193.673 ms = **331.254 ms**，预热后首编辑 114.067 ms；`perf_first_edit_after_first_frame` app 135.875 ms、首帧 184.362 ms、首编辑 1.232 ms。大头是一次性初始化而非稳态开销。
- 离散：10,003/19,999 块的 `index_rebuild` 与 `total` 中位指标 3 次极差 ≤5%；40,005 块 index p50 极差 6.0%、total p95 5.2%；`frame_compose` 在 10,003 档 6.8%、2,002 档低档抖动达 18.9%；2,002/5,005 块存在低档冷热态抖动（index p95 极差 18.4% / 21.8%，total p95 15.1% / 9.0%），中位数与 v1 的 5,005 档 index p95 相比 +8.3%，10,003/19,999 档差异 ≤1.0%；40,005 块档无 OOM/超时。

### 7.3 内存、线程与缓存

四档 × 3 次进程级采样（每 100 ms 一次，持续 12 s；稳定值、峰值、最终值三者一致）：

| 档位（词） | 文档字节 | Private（稳定 ≈ 峰值 ≈ 最终） | WorkingSet（稳定 ≈ 峰值 ≈ 最终） |
|---|---:|---:|---:|
| 10k | 68,923 | 36.12 MB / 37,875,712 | 53.44 MB / 56,037,376 |
| 50k | 344,630 | 36.94 MB / 38,735,872 | 54.12 MB / 56,745,984 |
| 100k | 689,280 | 37.89 MB / 39,727,104 | 55.09 MB / 57,765,888 |
| 200k | 1,378,561 | 39.55 MB / 41,467,904 | 56.83 MB / 59,588,608 |

交互后（`tools/walkthrough.ps1 -Perf -PerfChars 200`，`VEIL_PERF_PROBE=1`，`rss_mb` 粒度 1 MB）：

| 档位 | 样本数 | 中位 | 最大 | 最小 | 启动到窗口 |
|---|---:|---:|---:|---:|---:|
| 10k | 603 | 54 | 59 | 53 | 296 ms |
| 50k | 605 | 56 | 60 | 54 | 293 ms |
| 100k | 605 | 56 | 61 | 54 | 285 ms |
| 200k | 605 | 60 | 64 | 56 | 281 ms |

> 表中为 3 次运行的中位值；每次运行内的稳定值、12 s 峰值与最终值差异均 ≤0.02 MB（同档 3 次极差 Private ≤0.21 MB、WS ≤0.42 MB）。
- 20 倍词量（10k→200k）Private 仅 **+3.43 MB**、WorkingSet 仅 **+3.39 MB**，明显次线性；首帧后即进入平台期，12 s 内无增长。
- 200k 词峰值 56.83 MB WorkingSet / 39.55 MB Private，**远低于 150 MB** 预算。
- 历史单点 40.76 MB（2026-11-05，RSS @100k 词）与本表 100k 的 WorkingSet 55.09 MB **不可直接比较**：口径不同（RSS vs Windows `WorkingSet64`），且本轮新增 `PrivateMemorySize64`。

| 项 | 数值 | 证据等级 |
|---|---|---|
| `syntect` 语法集加载后 | **+7.1 MB**；渲染 1000 行 Rust 后 +22.2 MB | 历史引用（ADR-012） |
| 主线程 | winit 事件循环 + 排版/绘制 | 静态（架构事实） |
| `notify` 监听线程 | 1 个 | 静态（`platform/watch.rs`） |
| HTTP 抓取线程 | 每个抓取批次 1 个 | 静态（`platform/http.rs`） |
| 语法高亮后台线程 | 1 个（`bg_loop`，`thread::spawn`） | 静态（`view/highlight_bg.rs:55`） |
| 多实例 IPC 线程 | 仅第二实例启动时 | 静态（`shell.rs`） |
| 语法缓存 | 64 块 / 2,048 行（超限 LRU） | 静态（`view/highlight.rs:135-136`） |
| 定时唤醒 | 外部变更 300 ms；系统主题 1 s（仅跟随系统）；远程图片 100 ms（仅 pending）；光标闪烁 600 ms | 静态（`app.rs:44-50`） |
| 块尺寸上限 | 64 KiB / 512 行 | 静态（`view/layout.rs:83-84`） |
| 光栅像素上限 | 24,000,000 px | 静态（`view/layout.rs:86`） |
| 系统图像解码上限 | 100,000,000 px（WIC，约 400 MB RGBA） | 静态（`platform/wic.rs:55-56`） |
| HTTP 响应体上限 | 20 MiB | 静态（`platform/http.rs:19-20`） |
| 远程图片缓存 | **无数量上限**（`HashMap`，仅切文档时清空） | 静态（`view/cache.rs:227, 355-361`）→ **已知风险** |
| 文档本地图片缓存 | **无数量上限**（`HashMap`，仅按 `image_base` 代次 retain） | 静态（`view/cache.rs:222, 418`）→ **已知风险** |

---

## 8. 已否决的成熟组件（只引用，不重复论证）

| 候选 | 被评估模块 | 结论 | 依据 |
|---|---|---|---|
| `tree-sitter` | 代码高亮 | 未采用（体积 +800 KB，超出 10 MiB 余量） | `.trae/specs/component-replacement-wave/spec.md` 非目标 |
| `comfy-table` / `tabled` | 表格布局 | No-Go（字符网格与 I4 冲突为首要理由；+91,648 B 为成本项） | `docs/adr/ADR-013-table-layout.md`、§6.0.4 |
| `fuzzy-rapid` | Quick Open | 不存在（crates.io 无此包） | `docs/adr/ADR-014-quick-open-fuzzy.md`、spike §S5 |
| `fuzzy-matcher` / `nucleo` | Quick Open | No-Go（质量无净收益为首要理由；本轮 +24,576 B，旧探针区间上限 92.2 KB） | 同上 |
| `html2md` | HTML→Markdown | No-Go（GPL-3.0+ 许可证 + abort 不兼容 + 超门禁） | 本轮实测、§6.2 B-2 |
| `toml_edit`（整体替换配置解析） | 配置加载 | No-Go（行为变更未获批准） | spike §S2 |
| `typst-math` | 公式渲染 | No-Go（体积/依赖下界过大：+15,857,664 B、267 包） | `docs/adr/ADR-002-math-rendering.md` |
| `syntect`（采用） | 代码高亮 | Go（替换自研 17 语言状态机） | `docs/adr/ADR-012-syntax-highlighting.md`、spike §S1-b |
| `onig` / `onig_sys` | 代码高亮 | 否决（C 静态库与构建工具链） | ADR-012 |
| `rquickjs`+KaTeX / WebView2 / wasmtime | 公式渲染 | 否决 | ADR-002 |
| `egui` / `wgpu` / `rfd` | chrome / 渲染 / 文件对话框 | 否决或延后 | ADR-005、ADR-011 |
| `regex`（查找正则） | 查找 | 否决（替换后余量 −602,624 B；且非需求） | README 红榜、§6.2 B-6 |

---

## 9. 已知局限与处置

| # | v1 局限 | 处置 | 本轮证据 / 理由与触发条件 |
|---|---|---|---|
| 1 | cargo-bloat 是估算（LTO 合并/内联，±15%） | **Accepted** | 无法消除。缓解：候选决策全部改用独立探针实测边际；cargo-bloat 只用于模块归因，模块级数字仅作方向性判断 |
| 2 | 吞吐数据是单次运行 | **Resolved** | 5 档 × 3 次 + 子段 × 3 次，6 次运行全部 `exit=0`，取中位数；10,003/19,999 块的 `index`/`total` 3 次极差 ≤5%，40,005 块 index p50 6.0%、total p95 5.2%；`compose` 10,003 档 6.8%、2,002 档 18.9%；2,002/5,005 块的低档冷热态抖动使 index p95 极差达 18.4%/21.8%；与 v1 的 5,005 档 index p95 差异 +8.3%，10,003/19,999 档 ≤1.0% |
| 3 | 内存没有按组件拆分 | **进程级 Resolved；组件级 Deferred** | 四档 × 3 次：Private 36.12→39.55 MB、WorkingSet 53.44→56.83 MB（20 倍词量），次线性，200k 远低于 150 MB。字体/图片/布局缓存的独立占用仍无插桩。**触发条件**：做内存优化时，给 `view/cache.rs` 各类 `Memo` 加条目数与字节计数（已知 `cache.rs:222/227` 两个图片 `HashMap` 无数量上限） |
| 4 | B-5/B-7/B-8/B-9 增量是推断 | **Resolved** | 实测（含传递依赖、空探针口径）：`fluent-bundle` +224,256、`url` +221,696、`slug` +6,144、`clap` +293,376。这是“独立工程全量成本”，Veil 内是否已有共享代码未逐个核销；B-1 的 in-tree 边际仍**未确认** |
| 5 | `html2md` 的 2.29 MB 是在 `panic=unwind` 口径下测得 | **Resolved** | abort 下复验构建失败（错误原文见 §6.2 B-2）；同时确认 **GPL-3.0+**，许可证否决强于体积与 abort；v1 的体积值（unwind 基线 109,568）与本轮（基线 107,008 对齐后 +2,291,712）不可直接比较 |
| 6 | GUI present 路径未测 | **Deferred** | 需插桩：在 `softbuffer::Buffer::present()` 前后打时间戳（或 `DwmGetCompositionTimingInfo`/ETW），覆盖 1080p/4K × 60/120Hz × 大面积滚动；当前 `frame_compose` 到 present 之间无可观测点。**值得做的时机**：ADR-011 触发条件 1（4K/120Hz 掉帧）出现，或用户报告滚动不跟手 |
| 7 | <100 行模块未逐个判定 | **Resolved（3 个具名）/ Accepted（其余）** | `link_safety`（生产 167）、`slug`（63）、`cli`（97）已逐个读码并给出四维判定（§6.2）；其余 <100 行均为几何/常量/路径纯逻辑，无候选，按组接受 |

> 另有两项跨文档口径问题（不在 v1 七条内）：README.md:134 的体积数字滞后 104,960 B（本报告只标注）；ADR-002 的版本陈述自相矛盾（状态段写锁定 2.1.1，复核段写 3.0，§4.1 记 3.0.1），建议单独订正。

---

## 10. 附录：命令与原始数据

### 10.1 体积

```powershell
# 第三方与 std 归因（.text）
cargo bloat --release -p veil --crates -n 300

# 自研模块归因（函数级，按 veil_ui:: / veil_core:: / veil:: 前缀聚合）
cargo bloat --release -p veil -n 100000

# 生产 exe 与 PE 节
cargo build --release -p veil
(Get-Item target\release\veil.exe).Length   # 9,889,792
Get-FileHash target\release\veil.exe -Algorithm SHA256
dumpbin /headers target\release\veil.exe

# 依赖计数与直接依赖
cargo tree -e no-dev -p veil --prefix none
```

`.text` Top 15（crate，字节，占 exe；与 §3.2 同一基线）：

| crate | 字节 | 占 exe |
|---|---:|---:|
| std | 1,048,576 | 10.60% |
| veil_ui | 827,290 | 8.37% |
| latex_rust | 771,072 | 7.80% |
| skrifa | 651,776 | 6.59% |
| veil | 441,651 | 4.47% |
| regex_automata | 387,277 | 3.92% |
| harfrust | 377,242 | 3.81% |
| read_fonts | 350,720 | 3.55% |
| zeno | 208,998 | 2.11% |
| tiny_skia | 183,194 | 1.85% |
| veil_core | 172,339 | 1.74% |
| cosmic_text | 168,550 | 1.70% |
| regex_syntax | 151,859 | 1.54% |
| aho_corasick | 134,144 | 1.36% |
| pulldown_cmark | 130,560 | 1.32% |

### 10.2 运行期（性能）

```powershell
# 预编译后串行运行；5 档规模数组位于 app_tests.rs:8414 / :8524 的两个 #[ignore] 探针
cargo test -p veil-ui --release --lib --no-run
1..3 | ForEach-Object {
  cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_attribution
}
1..3 | ForEach-Object {
  cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_index_subsegments
}

# 端到端与冷启动（同日上一轮）
cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path
cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit
cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_first_edit_after_first_frame
```

### 10.3 运行期（内存）

```powershell
# 进程级采样：每 100 ms 取 WorkingSet64 / PrivateMemorySize64，持续 12 s
pwsh -NoProfile -File "C:\Users\31i45\AppData\Local\Temp\veil-memory\measure.ps1" `
  -Exe "d:\VSCode\Veil\target\release\veil.exe" `
  -Doc "C:\Users\31i45\AppData\Local\Temp\veil-memory\doc-10k.md" -Tag "10k"
# 同形调用：doc-50k / doc-100k / doc-200k，Tag 分别为 50k / 100k / 200k

# 交互后 RSS（脚本以 -Perf 自行设置 VEIL_PERF_PROBE=1）
pwsh -NoProfile -File "d:\VSCode\Veil\tools\walkthrough.ps1" -Doc "<doc>" -Perf -PerfChars 200
```

### 10.4 替换候选探针（工作区外，release/LTO/cgu=1/abort）

| 探针 | 空基线 | 接入后 | 增量 | 备注 |
|---|---:|---:|---:|---|
| `probe-fluent`（`fluent-bundle` 0.16.0） | 107,008 | 331,264 | **+224,256** | 24 个传递包 |
| `probe-url`（`url` 2.5.8） | 107,008 | 328,704 | **+221,696** | 46 个传递包（全平台口径） |
| `probe-slug`（`slug` 0.1.6） | 107,008 | 113,152 | **+6,144** | 2 个传递包 |
| `probe-clap`（`clap` 4.6.8） | 107,008 | 400,384 | **+293,376** | 16 个传递包 |
| `probe-json`（`serde_json` 1.0.154） | 107,008 | 168,960 | **+61,952** | 已在树（经 syntect），Veil 内边际未确认 |
| `probe-fuzzy`（`fuzzy-matcher` 0.3.7） | 107,008 | 131,584 | **+24,576** | 3 包；旧探针源码丢失，区间上限为 92,160 B |
| `probe-html2md`（0.2.17） | 107,008 | 2,398,720 | **+2,291,712** | **unwind 口径**；abort 下构建失败；GPL-3.0+ |

历史探针（未在本轮复跑，不用于决策）：

| 探针 | 增量 | 备注 |
|---|---:|---|
| `probe-table`（`comfy-table`） | +91,648 | ADR-013，字符网格与 I4 冲突 |
| `probe-base64`（编码边际） | +4,096 | v1 探针；A-1 实际执行后整包 +1,024（见 §6.1） |

### 10.5 本轮数据文件

| 文件 | 内容 |
|---|---|
| `%TEMP%\veil-spike\reassess-task1-task4.md` | 基线、门禁、依赖计数、候选边际探针 |
| `%TEMP%\veil-spike\reassess-task2.md` | 5 档性能、子段、三模型拟合 |
| `%TEMP%\veil-spike\reassess-task2-raw.csv` / `-raw.txt` | 逐次原始 CSV 与 stdout |
| `%TEMP%\veil-spike\reassess-task3.md` | 四档内存、交互 RSS、缓存上限核对 |
| `%TEMP%\veil-spike\reassess-task5.md` | 四维重判、像素契约备忘、局限处置 |
| `%TEMP%\veil-memory\doc-{10k,50k,100k,200k}.md` | 四档测试文档 |
| `%TEMP%\veil-memory\raw-<档>-run<N>.csv` | 12 次内存采样原始日志 |
| `%TEMP%\veil-memory\measure.ps1` | 进程内存采样脚本 |
| `%TEMP%\veil-spike\probe-{fluent,url,slug,clap}` | 4 个本轮候选探针工程 |
| `%TEMP%\veil-spike\probe-{json,fuzzy,html2md,empty}` | 对照与历史探针工程 |

### 10.6 关键源码定位

- 语言路由：`crates/veil-ui/src/view/highlight.rs`
- 主题解析：`crates/veil-ui/src/theme.rs`
- 表格：`crates/veil-ui/src/view/table_strategy.rs`、`table_view.rs`
- Quick Open：`crates/veil-ui/src/view/quick_open.rs`
- 块/行内解析：`crates/veil-core/src/syntax/{block,inline,mod}.rs`
- 排版/绘制：`crates/veil-ui/src/view/{layout,paint,frame,cache}.rs`
- 导出/PDF/打印：`crates/veil/src/{export.rs,platform/{pdf,pdf_layout,print}.rs}`
- base64（A-1 已执行）：`crates/veil/src/export.rs::base64_encode`、`crates/veil-core/src/mermaid.rs::base64url_encode`
- 手写 JSON：`crates/veil-core/src/callouts.rs`
- 手写 HTML→MD：`crates/veil-ui/src/html.rs`

---

## 11. v3.0.2 当前 HEAD 重新基线（2026-11-26）

> 本节是追加基线，供后续体积放行/否决使用。§1–§10 的 v2 结论与数字保留**历史身份**，不因本节改写；v2 的 `9.89 MB / 10 MiB` 不再作为当前版本的放行依据。

### 11.1 Task 8 基线卡

| 项 | 当前值 | 来源/口径 |
|---|---|---|
| HEAD | `71cb42a`（`git describe`：`v3.0.2-58-g71cb42a`） | 本机 `git rev-parse --short HEAD` |
| 版本 | `3.0.2` | `Cargo.toml` workspace.package.version |
| 构建 profile | `release`：`lto=true`、`codegen-units=1`、`panic=abort`、`strip=true`、`opt-level="s"` | `Cargo.toml`；与 Phase 2 证据一致 |
| 工具链 / 平台 | rustc/cargo 1.99.1；Windows 11 x64 | 本机 `rustc --version`；证据同 |
| 产物 | `target\release\veil.exe` **14,170,112 B**（13.51 MiB） | `Get-Item`；构建于 2026-11-26 12:48:55 |
| SHA-256 | `73A08EE9F0260AEFEA2C99ECC7BEB9EEF476E804E1908900362E4EFB05A5A3C3` | `Get-FileHash` |
| 体积门禁 | **≤ 14 MiB = 14,680,064 B**（发布门禁） | `README.md:208` |
| 对门禁余量 | **509,952 B（498.0 KiB）**，占门禁 3.47%；exe 已占门禁 96.53% | 14,680,064 − 14,170,112 |
| 门禁脚本固定基线 | 14,135,296 B；当前比该基线 **+34,816 B** | `docs/review/evidence/perf/perf-gate-2026-11-26.md` |
| 复测一致性 | 体积与 SHA-256 均与 Phase 2 证据逐字节一致 | `docs/review/evidence/perf/current-head-2026-11-26.md` §2/§4 |

> 本卡保留 Task 8 称重口径；下方“当前”均指 Task 8 基线。Phase 5 AccessKit 接入后的当前产物为 **14,362,624 B**、余量 **317,440 B**、SHA-256 见 §13。

**与 v2 的关系**：v2 基线为 `9,889,792 B @ ac25a9a`，门禁为 `10 MiB = 10,485,760 B`。`ac25a9a` 在当前克隆中不可检出（v3 历史已重写），其数字只作历史引用。当前 exe 比 v2 大 **+4,280,320 B（+43.28%）**，但两者的门禁与内容不同（v3 默认包含离线 Mermaid：完整 `mermaid.js` + QuickJS + resvg），因此**不直接比较通过/不通过或余量**，差值只用于 §11.3 归因。

### 11.2 PE 节对比（v2 → Task 8 基线）

> 本节节表是 Task 8 基线（14,170,112 B）的快照；Phase 5 接入 AccessKit 后的当前产物见 §13。

| 节 | v2（9,889,792 B） | Task 8 基线（14,170,112 B） | 增量 | 基线占 exe |
|---|---:|---:|---:|---:|
| `.text` | 7,228,928 | 7,826,944 | **+598,016** | 55.24% |
| `.rdata` | 2,462,720 | 6,067,712 | **+3,604,992** | 42.82% |
| `.data` | 2,048 | 3,072 | +1,024 | 0.02% |
| `.pdata` | 62,464 | 122,368 | +59,904 | 0.86% |
| `.rsrc` | 91,136 | 91,136 | **0** | 0.64% |
| `.reloc` | 41,472 | 57,856 | +16,384 | 0.41% |
| PE 头（`SizeOfHeaders`） | 1,024 | 1,024 | 0 | 0.01% |
| **合计** | **9,889,792** | **14,170,112** | **+4,280,320** | **100%** |

增量集中在 `.rdata`（+3,604,992 B，占增量 84.22%）与 `.text`（+598,016 B，13.97%）；`.rsrc` 完全未变，说明**图标没有增长**。

### 11.3 体积增量归因

| 归因项 | 字节 | 占 +4,280,320 | 证据等级 | 说明 |
|---|---:|---:|---|---|
| Mermaid 内嵌 JS 资源 | **3,213,314** | 75.07% | 实测（源文件字节） | 见下表；全部位于 `.rdata` |
| QuickJS 引擎、resvg 光栅栈及其他新增依赖 | **1,067,006** | 24.93% | **未细分** | 本 HEAD 尚无 crate 级 cargo-bloat 归因结论，无法在 rquickjs / resvg / regex / serde_json 等之间精确切分 |
| 字体 / 图标 | **0** | 0% | 实测（PE 节） | `.rsrc` 91,136 B 未变（两枚 ico 共 89,277 B 仍在内）；`tmTheme` 共 4,432 B 未变；仓库无新增字体文件；正文字体仍由 `fontdb` 扫描系统字体，公式字体仍随 `latex-rust` 编译 |
| AccessKit | **192,512（Phase 5 实测）** | 4.50%* | 实测 | 2026-11-26 起已加入（`accesskit 0.25.2` + `accesskit_winit 0.34.2`）；Task 8 基线时为 0，加入后总增量变为 **+4,472,832 B**；*百分比分母仍是 Task 8 的 +4,280,320 B，便于与本表各项对照。详见 §13 |

Mermaid 内嵌 JS 明细（`crates/veil-mermaid-offline/assets/`）：

| 资源 | 字节 | 用途 |
|---|---:|---|
| `mermaid.min.js` | 2,751,963 | 完整 mermaid 引擎（默认 feature 编译进 exe） |
| `linkedom.worker.mjs` | 421,658 | 无头 DOM（构建期转 IIFE 后 `include_str!`） |
| `bootstrap.js` | 39,693 | QuickJS 启动/桥接脚本 |
| **合计** | **3,213,314** | 占 exe 22.68%，占 v2→当前增量 75.07% |

- `crates/veil/Cargo.toml:8` 自述离线 Mermaid 功能“体积 +约 5 MiB”。这是**功能级估计**，不是隔离实测：本 HEAD 可复核的是整包相对 v2 增长 **4.08 MiB**，其中 JS 资源 **3.06 MiB**；其余 **1.02 MiB** 含 bundled QuickJS C 引擎、resvg/usvg、正则查找、callouts 的 `serde_json` 等，**未细分**。
- 新增直接依赖（静态）：`rquickjs 0.9.0`（bundled QuickJS，`veil-mermaid-offline`）、`resvg 0.50.0` 与 `tiny-skia-path 0.12`（`veil`）、`regex 1.13.2`（`veil-ui`，正则查找）、`serde` / `serde_json 1.0.154`（`veil-core`，callouts）。依赖树唯一包 **128（v2 记录）→ 162（本 HEAD 同口径）**，增量含 `rquickjs-sys`、`usvg`、`svgtypes` 等传递包。
- 已知重复：`resvg` 带入 `tiny-skia 0.12.0`，而 `latex-rust` 仍使用 `tiny-skia 0.11.4`，两份并存，相关代码无法完全去重。
- **AccessKit（Phase 5 已加入，实测 +192,512 B）**：历史预测为 **+185 KiB（189,440 B）**（非 spike 的 +198 KiB 预测），保留作对照；实测比预测多 **3,072 B（+1.62%）**。新产物 **14,362,624 B**，对 14 MiB 门禁余量 **317,440 B**。旧树 `bd3b03e` 的移除原因（只读标题、菜单无子节点、ActionRequested 不执行）已在本次接入中逐项解决，证据见 [accessibility-scope-and-limits.md](accessibility-scope-and-limits.md)。详见 §13。

### 11.4 B 类记录更新（截至当前 HEAD）

决策全部维持 v2 的 **Keep**；仅 B-1 由 `Spike-later` 更新为**已完成**。表中“替换后余量”以当前门禁余量 509,952 B 起算；边际值沿用 v2 独立探针（本 HEAD 未复跑），体积只作成本项。

| 编号 | 项 | 当前状态（HEAD `71cb42a`） | 决策 | 体积 / 依据 |
|---|---|---|---|---|
| B-1 | `callouts.rs` → `serde_json` | **已完成**：`veil-core` 直接依赖 `serde_json`；`from_json` 整文件解析后逐项提取，测试覆盖 emoji/代理对与 `\b`/`\f` | **Done（原 Spike-later 关闭）** | 实际增量**未细分**（已计入当前 exe；v2 探针 +61,952 B 为独立工程上限） |
| B-2 | `html.rs` → `html2md` | 仍手写；无 `html2md` 依赖 | **Keep** | GPL-3.0+ 许可证 + `panic=abort` 不兼容；+2,291,712 B → 替换后余量 **−1,781,760 B** |
| B-3 | `quick_open.rs` 排序 → `fuzzy-matcher` | 仍手写 `score/filter` | **Keep** | 质量负收益（`config` 2/10 vs 5/10）；+24,576–92,160 B，余量 417,792–485,376 B |
| B-4 | `theme.rs` 加载 → `toml_edit` | 仍为行解析 + `toml_edit` 写回的双解析器 | **Keep** | 0 边际，但 3 条容错语义相反，未获产品批准 |
| B-5 | `i18n.rs` → `fluent` | 仍 `keys!` 静态表、单语言 | **Keep** | +224,256 B → 余量 285,696 B；无复数/日期需求 |
| B-6 | `search_strategy.rs` 字面搜索 → `regex` | 字面 `memchr` 路径保留；`regex 1.13.2` 现为直接依赖，作为可选正则模式（`README.md:108`），**不是**对字面搜索的替换 | **Keep** | v2 的“`regex` 未链接 / +1,198,592 B”前提已过时；字面搜索仍是默认需求 |
| B-7 | `link_safety.rs` → `url` | 仍白名单 + 实体/百分号去混淆 | **Keep** | +221,696 B → 余量 288,256 B；`url` 只覆盖 scheme 语法，安全语义更弱 |
| B-8 | `slug.rs` → `slug` | 仍自研、CJK 原样保留 | **Keep** | +6,144 B → 余量 503,808 B；音译会改变既有锚点身份 |
| B-9 | `cli.rs` → `clap` | 仍手写；仅 `--new-window`、`--`、单位置参数 | **Keep** | +293,376 B → 余量 216,576 B；1 个 flag 属过度 |

### 11.5 复现

```powershell
cargo build --release --bin veil
(Get-Item target\release\veil.exe).Length            # 14,362,624（Task 8 基线为 14,170,112）
Get-FileHash target\release\veil.exe -Algorithm SHA256
dumpbin /headers target\release\veil.exe             # .text / .rdata 等节
cargo tree -e no-dev -p veil --prefix none           # 唯一包计数
```

证据引用：`docs/review/evidence/perf/current-head-2026-11-26.md`（体积/SHA/门禁）、`docs/review/evidence/perf/perf-gate-2026-11-26.md`（自动门禁与固定基线）、`README.md:208`（14 MiB 门禁）。本节的 v2 对照数字来自 §3.1，未重新生成。

---

## 12. Phase 4 决策回写（2026-11-26）

> 本节追加 Phase 4（Task 9 / 10 / 11）的决策，供后续集成与体积放行引用；§1–§11 不改写。基线仍是 Task 8 的 `14,170,112 B / 14 MiB 门禁 / 509,952 B 余量`。

### 12.1 PDF 断行子模块：**NO-GO**，保留自定义断行

| 项 | 结论 | 依据 |
|---|---|---|
| 候选 `unicode-linebreak` 0.1.5 | **NO-GO** | 8 组语料共 118 行，7 组逐行完全一致；仅 `mixed` 2 行差异（当前 28.82em 止于词边界，候选 42.82em 挤满），候选无质量净收益 |
| 行为契约 | **不通过** | 候选只接受 `&str`，丢失 `LItem`（链接开闭/图片盒/样式）与 `last_space` 偏好；替代需重写行装配并重跑 Task 7 的 52 项回归 |
| 体积 / 许可证 / 离线 / 无 webview | 通过但无意义 | 仅 dev-dependency，不进入 release；Apache-2.0、0 传递依赖；该 crate 已由 `cosmic-text` 进入生产依赖图 |
| `icu_segmenter` 2.4.0 | **不引入** | 数据 crate `icu_segmenter_data` 压缩 3,141,236 B（≈3.14 MB），当前余量仅 509,952 B；至少 7 个必需 ICU4X crate；Unicode-3.0 兼容但需同步 notice/SBOM |

- 已知缺口不因换候选而消失：`—` 在 UAX #14 属 B2（前后可断），当前与候选都会把它放到行首；屏幕靠 WJ 禁止，PDF 侧仍待裁决。
- 处置：不修改生产代码；`pdf_layout.rs` 仅新增 `#[cfg(test)]` spike 挂载；保留 dev-dependency 与 2 个可复现测试（`cargo test -p veil pdf_linebreak_spike -- --nocapture`）。release 产物仍为 **14,170,112 B**；Task 9 重建的 SHA-256 与 Task 8 不同（PE 时间戳等构建元数据），门禁按尺寸判定，不做逐字节回归。
- 再评估触发（任一）：① 需为泰/老/高棉/缅等无空格语言做词边界断行；② 决定用 UAX 断点统一屏幕与 PDF 的单一真值层；③ 需处理超过 2 个连续闭符号的避头或 UAX 数值语境；④ `cosmic-text` 升级导致屏幕 UAX 表变化、需要独立断行 oracle。
- 证据：`docs/review/evidence/spike/pdf-linebreak.md` §3/§6/§7。

### 12.2 `windows` / `image`：只观察，不引入

| 组件 | 当前事实 | 决策 | 再评估触发（摘要） |
|---|---|---|---|
| `windows` / `windows-registry` | 依赖图中**没有** `windows` 包；生产手写 Win32/COM FFI：79 条 `extern "system"`、16 个 COM 槽位（`windows.rs` 9 + `wic.rs` 7）；`windows-sys` 0.52/0.59/0.61.2 仅为传递的原始绑定 | **不引入，只观察** | FFI 面积继续增长（>约 100 个 extern 或第 3 个 COM 接口族）；出现可归因于 vtable/`repr(C)`/生命周期的崩溃或 UB；需要成批新 API（UIA、DWM 扩展、Shell 扩展等）；Phase 5 AccessKit 把 `windows 0.62` 带入构建；需要跨版本/架构的 ABI 审计证据 |
| `image` | `image` 0.25.10 **已由 `arboard 3.7.0` 传递引入**，但仅启用 `bmp` + `png` feature；PNG 走 `png` 0.18.1，非 PNG 走系统 WIC（JPEG/BMP/GIF/TIFF 由系统 codec 覆盖，WebP 依赖机器已装 codec；只取第 0 帧，上限 1 亿像素） | **不引入新 feature，只观察** | 产品要求不依赖系统 codec 的多格式确定性解码；WebP/AVIF/HEIC 在无 codec 机器打不开，或需要 GIF 动画；WIC 的 EXIF/颜色/alpha 缺陷无法在参数层修复；导出格式扩展 |

- **订正**：此前的“仅 PNG 解码”只对**纯 Rust 路径**成立；非 PNG 实际已由系统 WIC 覆盖。`image` 的收益不是“从 1 种格式到 N 种”，而是把已支持的格式从系统 codec 换成进程内确定性解码。
- 体积余量仅 509,952 B；任何替换先按六道门做独立 spike 称重，不直接进生产。
- 证据：`docs/review/jobs-lens-observation-windows-image.md` §1/§2.3/§3.5/§4.3。

### 12.3 B 类保留理由（Phase 4 定稿）

| 项 | 决策 | 保留理由（摘要） |
|---|---|---|
| `quick_open` 排序 → `fuzzy-matcher` | **Keep** | 手写排序质量更高（`config` precision@10 5/10 vs 2/10），ADR-014；+24,576–92,160 B 无质量回报 |
| `html.rs` → `html2md` | **Keep** | **GPL-3.0+** 许可证 + `panic=abort` 不兼容；+2,291,712 B 使余量变为 **−1,781,760 B** |
| `theme` 读取 → `toml_edit` | **Keep** | 写回已用 `toml_edit`；整体替换会让非法值/重复键/旧 `[theme]` 段 3 条容错语义反转，未获产品批准 |
| `i18n` → `fluent` | **Keep** | 单语言纯插值，无复数/日期需求；+224,256 B、24 包无收益 |
| 字面搜索 → `regex` 替换 | **Keep** | 字面 `memchr` 路径仍是默认需求；`regex 1.13.2` 已作为可选正则模式直接链接（`README.md:108`），不是替换 |
| `link_safety` → `url` | **Keep** | `url` 只覆盖 scheme 语法，实体/百分号去混淆与白名单仍须自留，安全判定反而更弱；+221,696 B |
| `slug` → `slug` | **Keep** | deunicode 音译会把中文标题改成拼音，改变所有既有锚点身份；+6,144 B 但净收益为负 |
| `cli` → `clap` | **Keep** | 仅 `--new-window`、`--`、单位位置参数；+293,376 B、16 包属过度 |
| `callouts` → `serde_json` | **已完成（B-1 Done）** | 已删除手写 parser，补齐代理对、标准转义与可定位错误；保留 `validate` 薄封装 |

- 体积口径仍以 §11 的 509,952 B 余量为分母；边际值沿用 v2 独立探针，本 HEAD 未复跑。B-2 是唯一超门禁项，B-9/B-7 替换后余量分别降至 216,576 B / 288,256 B，均属成本项而非首要否决理由。

### 12.4 与 follow-ups 的一致性

- **D3**：**已于 2026-11-27 撤销**。AccessKit/UIA 层在提交 `3a6d72d` 被永久移除（删 666 行、`Cargo.lock` 不再含 `accesskit`），产品**不提供**屏幕阅读器 / UIA 支持；本节 §13 与 [accessibility-scope-and-limits.md](accessibility-scope-and-limits.md) 的「已接入」描述仅作历史记录。
- **4.1** 已回写为 **NO-GO**；**4.2 / 4.3** 已标记观察备忘录完成、不引入（含 `image` 由 `arboard`/WIC 覆盖的订正）；**4.4 / 4.5** 保持已完成/已记录。详见 `.trae/specs/evaluate-veil-through-jobs-lens/follow-ups.md`（该目录被 `.gitignore` 忽略，不入库）。

---

## 13. Phase 5 实际称重：AccessKit 接入（2026-11-26）

> 本节替代 §11 中“Phase 5 预估/未加入”的口径；历史预测保留在 §11.3 作对照。

### 13.1 产物

| 项 | 值 |
|---|---|
| 提交 | `753c962`（`feat(a11y): expose navigable AccessKit tree for chrome and body`）；本文档提交在其后 |
| 构建 | `cargo build --release -p veil`（默认 feature 含 `accessibility`） |
| 产物 | `target\release\veil.exe` **14,362,624 B**（13.70 MiB） |
| Task 8 基线 | 14,170,112 B |
| AccessKit 实测增量 | **+192,512 B**（188.0 KiB） |
| 历史预测 | +185 KiB（189,440 B）；实测多 **+3,072 B（+1.62%）** |
| 14 MiB 门禁 | 14,680,064 B |
| 门禁余量 | **317,440 B**（310.0 KiB），占门禁 2.16% |
| 总增量（v2 → 当前） | +4,472,832 B |
| SHA-256 | `1D456A3D532CEC10A8D141592281D502E88A419BFCC22EC3CB8CB0D0BD819EC3` |

### 13.2 依赖与开关

- `accesskit =0.25.2`、`accesskit_winit =0.34.2`（`default-features=false, features=["rwh_06"]`），均为 optional。
- `veil-ui` 自身默认关闭 `accessibility`；`veil` 默认 feature 显式开启，发行产物包含。
- 关闭：`cargo build --release -p veil --no-default-features --features mermaid-offline`。

### 13.3 复现与证据

```powershell
cargo build --release -p veil
(Get-Item target\release\veil.exe).Length            # 14,362,624
Get-FileHash target\release\veil.exe -Algorithm SHA256
```

- 体积门禁余量按尺寸判定，不做逐字节回归；SHA 随重建变化，以本表为准。
- 功能与限制：[accessibility-scope-and-limits.md](accessibility-scope-and-limits.md)。
- UIA 实测：[evidence/ux/accesskit-uia-2026-11-26.md](evidence/ux/accesskit-uia-2026-11-26.md)。

### 13.4 IME 锚点与用户反馈修复后的再称重（2026-11-26 追加）

| 项 | 值 |
|---|---|
| 变更 | `ImeAnchor` 端口 + `imm32` 手写绑定（`crates/veil/src/platform/ime.rs`）；preedit 不透明底与强调色下划线；导出 CSS 去硬限高；任务框改同描边画法；无新增第三方 crate |
| 产物 | `target\release\veil.exe` **14,363,648 B** |
| 相较 §13.1 | **+1,024 B** |
| 14 MiB 门禁 | 14,680,064 B |
| 门禁余量 | **316,416 B**（309.0 KiB） |
| SHA-256 | `1D0A11427C1CC8FD363F9C17B0FC8AC90FC46B1896E06D7BD30EC0AD3C30E475` |

> `imm32` 为系统导入库，不增加 exe 体积；增量来自新增端口虚调用与绘制代码。

