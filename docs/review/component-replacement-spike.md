# 内容层组件替换 Spike 记录

日期：2026-11-16
规格：`.trae/specs/replace-content-components/`
口径：Windows 11 x64、Rust 1.99.1、`--release`、`lto=true` / `codegen-units=1` / `panic=abort`、串行测量。

## 基线（替换前）

| 项 | 值 | 命令 |
|---|---|---|
| `veil.exe` | 7,761,408 B | `(Get-Item target\release\veil.exe).Length` |
| workspace 传递依赖（去重） | 101 | `cargo tree -e normal --workspace --prefix none` |
| 测试 | 全绿（veil-ui 1127 passed / 11 ignored，其余 crate 同轮全绿） | `cargo test --workspace` |
| clippy | 零警告 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 击键 p95（5,005 / 10,500 块） | 1.234 ms / 1.773 ms | `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke` |
| 冷启动 + 首次编辑 | 142.922 + 156.956 = 299.878 ms；预热后 116.877 ms | 同上，`perf_cold_first_edit` |
| 常驻内存（10 万字） | 40.76 MB | README 数字红线（2026-11-05 实测） |

## S1：syntect（代码高亮）

spike：本地原型 `syntect-size`（fancy-regex）与 `syntect-onig`（onig），同一 10 语言 × 1000 行固定语料（原型目录不入库，已清理）。

| 指标 | 现有自研状态机 | syntect 5.3.0 `default-fancy` | syntect 5.3.0 `default-onig` |
|---|---|---|---|
| exe 增量（相对 hello 111,616 B） | 0 | +1,986,048 B | +1,045,504 B |
| `SyntaxSet` 冷加载 | 0 | 1.1 ms | 1.0 ms |
| 语法数 | 19 个规范名 | 75 | 75 |
| 10,000 行逐行解析总耗时 | 约 5.9 ms（按 1000 行 0.586 ms 推算） | 1,076 ms | 381 ms |
| 单行 p50 / p95 / p99 | — | 46 / 229 / 360 µs | 25 / 99 / 132 µs |
| 依赖（spike 子树去重） | 0 | 48 | 47 |
| 1000 行 Rust 代码块 | **0.586 ms**（p95 0.636 ms） | **152.43 ms** | **42.15 ms** |
| 常驻内存（10k 行后） | 可忽略 | 106,512 KB | 17,408 KB |

同语料下 1000 行各语言 onig 耗时：Rust 42.15 / C 37.95 / Python 50.05 / JavaScript 42.13 / Java 31.30 / Go 20.98 / C++ 50.38 / Ruby 36.61 / Bash 24.97 / YAML 17.97 ms。

许可证：`syntect` MIT、`onig`/`onig_sys` MIT、`fancy-regex` MIT，均兼容。

结论：**初测 No-Go，S1-b 复核后改判 Go**。

- 初测假设「每次布局重算整块」。复核确认布局层已有因果键缓存（ADR-009），只有被编辑块/光标进出块才重排；且 syntect 支持缓存 `ParseState` 从改动行重启。该假设不成立，见下节 S1-b。
- 体积不是阻塞项：fancy 变体增量约 1.99 MiB，仍在 10 MiB 目标内。
- onig 需要 C 静态库与构建工具链，纯 Rust 的 fancy 后端在增量模型下达标，选 fancy。

## S1-b：增量高亮复核（fancy）

spike：本地原型 `syntect-size` 的 `incr_bench`（已清理）。按行缓存 `ParseState`：行文本与行首状态一致即复用，否则从该行重解析。

| 指标 | 结果 |
|---|---|
| `SyntaxSet` 冷加载 | 0.9 ms（`OnceLock` 惰性，首次遇到代码块才加载） |
| 100 行块单字符编辑 p95 | 0.168 / 0.232 / 0.185 ms（行首 / 中间 / 行尾） |
| 1000 行块单字符编辑 p95 | 0.478 / 0.449 / 0.393 ms（行首 / 中间 / 行尾） |
| 重解析行数 | 中位数 1，最大 2–4 |
| 首次完整高亮 | 20 行 49.9 ms（含预热离群）、50 行 5.3、100 行 10.2、200 行 19.4、500 行 52.7、1000 行 101.4 ms |
| 内存增量 | 加载后 7.1 MB；1000 行 Rust 后 22.2 MB |
| exe 增量 | +2,023,424 B（集成后实测 9,784,832 B，10 MiB 内，余量 700,928 B） |
| workspace 传递依赖 | 154（基线 101，门禁 200） |

结论：**Go**。编辑路径 p95 < 0.5 ms，远低于 16 ms 击键门禁；>200 行块首次显示有一次性 20–100 ms 卡顿，已写入 ADR-012 后果与触发再评估条件。缓存上限（64 块 / 2048 行）保证内存有界。

## S2：toml_edit（配置）

spike：本地原型 `toml-edit`（已清理），toml_edit 0.22.27。

| 样本 | 现有手写解析 | toml_edit |
|---|---|---|
| 当前生成文件 | 正常 | 正常，`load → save` 字节一致 |
| 注释 + 未知键 + 未知表 | 正常 | 正常，编辑后注释/未知键/顺序保留 |
| 非法值（`mode = `） | 该键计入 ignored，其余键继续生效 | **整文档解析失败** |
| 重复键（`mode` 两次） | 后者生效 | **整文档解析失败**（TOML 规范禁止） |
| 历史 `[theme]` 段 | 段标题跳过，段内 `mode` 仍按顶层生效 | 解析为嵌套表 `theme.mode`，顶层无 `mode` |

体积增量：+232,448 B；依赖子树 8 个 crate（与 workspace 已有部分重叠）。

结论：**No-Go（未获行为变更批准）**。

- 注释与未知键保留是真实收益，但代价是改变三条既有容错语义：非法值逐键忽略、重复键后者生效、`[theme]` 段兼容。
- 规格要求「必须先取得明确批准；未批准则 No-Go」。本次未获单独批准，故保留手写实现。
- 若未来只想要「保存时保留注释」，可作为独立变更提案：加载继续用现有容错解析，保存前用 `toml_edit` 编辑已存在文档，解析失败时回退整文件重写。该方案不改变加载语义，但会引入第二条写入路径，需要单独评审。

## S3：typst-math（公式）

spike：本地原型 `math-compare`（已清理），只链接 `typst` 0.15.1 + `typst-layout` 0.15.1 的编译器与排版路径（`typst::compile::<PagedDocument>`）。**不含字体（typst-assets）与 LaTeX→Typst 转换（mitex）**，因此是体积与依赖的下界。固定 20 条样本清单随原型删除，结论与数字保留在本文与 ADR-002。

| 指标 | 结果 |
|---|---|
| exe 增量（相对 hello 111,616 B） | +15,857,664 B（≈15.1 MiB），仅代码，不含字体 |
| 叠加到现有 Veil | 7,761,408 + 15,857,664 = 23,619,072 B（≈22.5 MiB），已超 20 MiB 硬门禁 |
| 新增依赖（normal 边去重） | 267（仅 typst 子树；workspace 当前 101，门禁 200） |
| release + LTO 构建时间 | 16 分 24 秒（本机） |
| 质量 / 单条耗时对比 | 未进入：体积与依赖已一票否决，按 1 人日时限不再做渲染对比 |

结论：**No-Go，保留 `latex-rust`**。

- 体积下界已超 20 MiB 硬门禁；真实集成还要嵌入数学字体并加入 LaTeX 语法转换，只会更大。
- 依赖子树 267 个 crate，单这一项就会把 workspace 传递依赖从 101 推到约 350，突破 200 门禁。
- 裁决不依赖排版质量：这是契约门禁失败，不是质量比较失败。
- 若未来把「出版社级公式排版」列为一等目标，必须同时放宽体积与依赖门禁并单独立项；不能以「质量更好」为由绕过门禁。

## S4：comfy-table（表格布局）

日期：2026-11-17。规格：`.trae/specs/component-replacement-wave/` Task 4。
spike：`%TEMP%\veil-spike\probe-table`（独立工程，`comfy-table` 8.0.1，`default-features = false` 关闭终端 tty/crossterm，与本项目自绘场景一致）。
基线：同一台机、同一 release 配置的空探针 `probe-empty` = 107,008 B。

| 指标 | 结果 |
|---|---|
| exe 增量（空探针 107,008 B → 198,656 B） | **+91,648 B** |
| 当前 `veil.exe` 余量 | 11,232 B（9,888,768 / 9,900,000）→ 增量是余量的 **8.2 倍** |
| 新增 unique 包 | 1（`comfy-table`；`unicode-width 0.2.2`、`unicode-segmentation 1.13.3` 已在 workspace 树中） |
| 被评估的自研代码 | `view/table_strategy.rs` 1,706 行 + `view/table_view.rs` 829 行 = 2,535 行 |

能力对照（读 comfy-table 8.0.1 API 与本项目实现）：

| 需求 | Veil 现状 | comfy-table |
|---|---|---|
| 列宽来源 | cosmic-text 逐字形测量（DPI/字号/字体） | `unicode-width` 字符格宽，无像素概念 |
| 单元格换行 | 禁则感知折行 + cosmic-text shaping + 样式段映射 | 按字符数贪心折行，输出纯 `String` |
| 单元格富文本 | 粗体/斜体/行内代码/链接色/删除线逐段样式 | 仅整格 ANSI 前景/背景色 |
| 行内公式 / 图片 | NBSP 占位 + 字形锚点 + 原位 blit | 无此概念 |
| 光标落点 / 命中 | 字形 → 文档字节偏移档位 + 网格命中 | 无（终端输出，不接收坐标） |
| 网格绘制 | softbuffer 像素级 1px 线、DPI 对齐、列宽拖拽 | 字符画边框（`+---+`） |
| 活动单元格源码态 | 含标记源码 + caret map | 无 |

结论：**No-Go，保持自研表格**。

- comfy-table 的输出契约是「终端字符网格字符串」，Veil 的输出契约是「像素几何 + 逐格光栅位图 + 字形锚点 + 字节偏移映射」。桥接只能复用「边框字符排布」这一层，而 Veil 的边框是像素级绘制，连这一层也用不上；实际复用率约为 0。
- 二者在列宽/折行上必须双份计算：先用 cosmic-text 量出像素宽，再折算成字符格喂给 comfy-table，最后仍要按 comfy-table 的字符网格重新反推像素位置——增加一层有损换算，而不是减少代码。
- 体积一票否决：+91,648 B 对 11,232 B 余量，即使未来只想要「边框画法」也必须先有 8 倍余量。
- 若未来需要终端导出（例如把表格打印成纯文本），可把 comfy-table 仅作为**导出路径**的可选依赖单独立项；渲染主路径不引入。

## S5：Quick Open 模糊匹配

日期：2026-11-17。规格：`.trae/specs/component-replacement-wave/` Task 5。
事实核查：**`fuzzy-rapid` 在 crates.io 不存在**（`cargo info fuzzy-rapid` → `could not find`；`cargo search` 无结果）。改用两个最接近的成熟替代做对比：`fuzzy-matcher` 0.3.7（MIT，skim 算法，TUI 生态常用）与 `nucleo` 0.5.0（MPL-2.0，Helix 的匹配引擎，异步流式设计，对 ≤500 条候选属于过度设计，未纳入基准）。

spike：`%TEMP%\veil-spike\probe-fuzzy`，243 条真实形态候选（20 个目录 × 12 个文件名 + 3 个最近文件），Veil `score()`/`filter()` 原样复制。查询 `rust` / `python` / `config`。

排序质量（文件名优先、全路径回退；同名不同目录去重后取前 10；`precision@10` = 与查询明确相关的文件名数）：

| 查询 | Veil precision@10 | fuzzy-matcher(SkimV2) precision@10 | 说明 |
|---|---|---|---|
| `rust` | 4/10 | 4/10 | 两者都把 `rust.md` + 3 个 `rust-*` 排在最前 |
| `python` | 4/10 | 4/10 | 同上，`python.md` + 3 个 `python-*` |
| `config` | **5/10** | 2/10 | Skim 把大量同分项按出现顺序铺开，`app-config` 被挤出前排 |

| 指标 | Veil 手写 | fuzzy-matcher 0.3.7 |
|---|---|---|
| 单次过滤（243 候选） | 144.8 µs | 73.2 µs |
| 500 候选（`CANDIDATE_CAP`）折算 | ≈0.30 ms | ≈0.15 ms |
| 击键门禁 16 ms | 通过 | 通过 |
| exe 增量（空探针 107,008 → 199,168 B） | 0 | **+92,160 B**（余量的 8.2 倍） |
| 新增 unique 包 | 0 | 2（`fuzzy-matcher` 本身 + `thread_local`；`cfg-if` 已在树中） |

结论：**No-Go，保持手写**。

- 目标组件不存在：`fuzzy-rapid` 是规格中的占位名，crates.io 无此包，无可替换对象。
- 质量没有净收益：`config` 查询下 Skim 的前排精度反而更低（2/10 vs 5/10）；`rust`/`python` 持平。Veil 的词首/连续/长度惩罚 + 最近文件加成是贴合「文件路径候选」的，通用匹配器并不天然更好。
- 速度差异不影响体验：0.30 ms vs 0.15 ms 都在击键路径噪声内。
- 体积一票否决：+92,160 B 对 11,232 B 余量。
- 可吸收的局部改进（不引依赖）：若后续需要，可在手写实现内引入 Skim 的「最优子序列 + 连续/词首权重」动态规划，纯代码改动，零体积代价；但这属于排序微调，不属于组件替换。
