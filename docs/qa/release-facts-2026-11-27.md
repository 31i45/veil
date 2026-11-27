# Veil 交付事实（2026-11-27）

> 本页为人工维护的交付事实。`report-2026-11-27.md` 由语法矩阵测试生成，运行测试会覆写，请勿手工追加。

## 直接修复轮（2026-11-28）

> 以下数字来自 2026-11-28 07:21–07:33（Asia/Shanghai）的本机真实命令输出。
> 源码改动为 `2060da9` / `67cdd8d` / `85b0953` 三个 commit；本轮文档收口本身未提交、未推送。

### 全量门禁

- `cargo test --workspace`：**2269 passed / 0 failed / 18 ignored**（分套件汇总 2269；`veil-mermaid-offline` 的病态 deadline 测试另以子进程两次复跑同一二进制，输出各多 1 passed，故结果行合计 2271 passed）。
  - 分套件：`veil` lib 288（1 ignored）、`veil` bin 0、`docs_policy` 6、`syntax_matrix` 5（306.58 s，内部 2148 用例 × 7 表面）、`veil-core` lib 368（1 ignored）、`conformance` 1、`core` 86、`gfm_conformance` 1、`parser_differential` 2、`real_world_check` 1、`pathological` 0（1 ignored）、`veil-mermaid-offline` 29（1 ignored）、`veil-ui` lib 1475（14 ignored）、`boundary_guard` 7、doc-tests 0×4。
- `cargo clippy --workspace --all-targets -- -D warnings`：**0 warnings**（exit 0）。
- `cargo fmt --all --check`：**通过**（exit 0）。
- `pwsh -NoProfile -File tools/verify/probe-gate.ps1`：**PASS / FAIL 0**，构建 2.2 s、运行 38.4 s；归档 `docs/qa/probe-gate-2026-11-28.md`。本轮一次通过，无偶发。

### 探针实测（本轮门禁）

- `perf_5000_headings`：打开 93.581 ms、滚动 p95 0.940 ms。
- `perf_history_5000`：打开 4.914 ms、滚动 p95 3.322 ms、关闭 0.391 ms。
- `perf_huge_block`（首键 p95 / 文末插入 p95）：1 MB **46.436 / 42.965 ms**（上限 70）、2 MB **53.256 / 50.864 ms**（上限 80）、10 MB **147.227 / 153.297 ms**（上限 200）。

### 三项直接修复

- **Callout 内 `<details>` 与普通引用同规则**（`2060da9`）：删除 `block.rs` 中 `(!is_callout).then_some(depth)` 的容器特判，Callout 与普通引用共用同一提升路径；`ADJC2-061/083` 金样、`adjacency-digest.txt`、`report-2026-11-27.md` 同步更新，Callout 标题只出现一次。
- **10 MB 单块首键延迟**（`67cdd8d`）：归因显示主因是每次击键把整块 10 MB 重新 inline 解析（约占 82%）；改为只解析需渲染前缀、把光栅限制在可见行带。实测 10 MB 首键 p95 **976.024 ms → 147.227 ms**（文末插入 975.732 → 153.297 ms）；1 MB 首键 p95 160.465 → 46.436 ms。预算写进 `perf_huge_block` 测试内：1 / 2 / 10 MB = **70 / 80 / 200 ms**（regression ceiling，非目标值），既有不变量断言未弱化。
- **ignored 门禁简化**（`85b0953`）：预算回归下沉到测试自证；`probe-gate.ps1` 由 **694 行降到 123 行**，只做 release 构建 + 一次 `--ignored` 运行 + 归档；删除 `perf-baseline.json`、`perf-baseline-final.json` 与 3 份 stability 归档。

### 发布产物与不变量

- `target/release/veil.exe` = **14,312,448 B**（门禁 14,680,064 B，余量 367,616 B）。
- SHA-256 = `3B567E860AE2337393261502079140DEC6F316151F0B6236719165AED57928FC`。
- 未推送 GitHub（`main` 领先 `origin/main` 140 个 commit）、未打 tag、版本号仍 **3.0.2**。
- 未新增依赖：`Cargo.toml` / `Cargo.lock` 自 `3ee2dcd` 起无改动。
- 本轮 commit（新→旧）：`85b0953`、`67cdd8d`、`2060da9`。

### 人工证据（2026-11-28）

- **系统 DPI 100% / 125% / 200%**：真实切换系统缩放后逐档抓首帧，四档均 `status=PASS`；窗口物理尺寸 916×739 / 1143×922 / 1372×1106 / 1826×1335，随缩放线性变化；首帧文字清晰、无空白/半屏/裁切。逐档点按/拖选命中 **未执行**，不冒充通过。证据：`docs/qa/evidence/assisted-2026-11-27/dpi-{100,125,150,200}.png`、`dpi.jsonl`。
- **真机微软拼音六步**：真实按键驱动，预编辑/空格上屏「你好」/`zhongguo` 第 2 候选「中过」/行首行尾/表格单元格/`Esc` 哈希不变均通过；微软五笔未安装 → NOT RUN。观察：组合中失焦会把原始串 `nihao` 提交进文档（系统 IME 语义，不丢字、不重复）。证据：`ime-result.md` 与 `ime-*.png`、`ime2-*.png`、`ime3-*.png`。
- **实体打印**：可选冒烟，NOT RUN；Print-to-PDF 已覆盖应用侧分页正确性。

## 最后一公里与 ignored 门禁（2026-11-28）

> 本节为 Task 1–6 轮次的历史记录；其中探针门禁口径、体积与 SHA 已被上方「直接修复轮（2026-11-28）」取代。
> 以下数字全部来自 2026-11-28 02:04–02:31（Asia/Shanghai）的本机真实命令输出。

### 全量门禁

- `cargo test --workspace`：**2269 passed / 0 failed / 18 ignored**（按套件汇总为 2267 passed；`veil-mermaid-offline` 的病态 deadline 测试以子进程两次运行同一二进制，额外 2 passed）。
  - 分套件：`veil` lib 288（1 ignored）、`veil` bin 0、`docs_policy` 6、`syntax_matrix` 5（311.30 s，内部 2148 用例 × 7 表面）、`veil-core` lib 366（1 ignored）、`conformance` 1、`core` 86、`gfm_conformance` 1、`parser_differential` 2、`real_world_check` 1、`pathological` 0（1 ignored）、`veil-mermaid-offline` 29（1 ignored）、`veil-ui` lib 1475（14 ignored）、`boundary_guard` 7、doc-tests 0×4。
- `cargo clippy --workspace --all-targets -- -D warnings`：**0 warnings**（exit 0）。
- `cargo fmt --all --check`：**通过**（exit 0）。
- `pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 9`：**PASS 20 / FAIL 0 / SKIP 0**，探针总耗时 399.2 s；归档 `docs/qa/probe-gate-2026-11-28.md`。（历史口径：`-Repeat` 已随 `85b0953` 移除，现门禁一次跑完全部 ignored 探针。）
- 环境性偶发：本轮全量与三次稳定性复跑均未复现早前 `platform::http::tests::actual_loopback_peer_fails_closed` 的 WinHTTP 12152；该历史记录保留在本页「测试」一节。稳定性验证中曾有一次 `perf_after_edit_breakdown` `peak_total_ms` FAIL（p50 5.96 ms vs 阈值 5.613 ms），单探针 9 次复测 p50 3.812 ms，随后连续三次 PASS；该次未改写成 PASS。

### ignored 清单

- 源码 `#[ignore]` 真实计数：**18**（`rg -n "#\[ignore" crates --glob '*.rs'`）。
- 名单：`pathological_eval_child`、`pathological_inputs_matrix`、`bench_edit_reparse_history_100k`、`perf_50mb_long_doc`、`perf_20k_blocks`、`perf_keystroke_path`、`perf_keystroke_attribution`、`perf_index_subsegments`、`perf_cold_first_edit`、`perf_cold_first_edit_prewarmed`、`perf_first_edit_after_first_frame`、`perf_huge_scroll`、`perf_huge_block`、`perf_peak_breakdown`、`perf_after_edit_breakdown`、`generate_wave4_sample_pdf`、`perf_5000_headings`、`perf_history_5000`。
- `mindmap_viewbox_debug` 已晋级为常规断言（不再 ignore）；probe-gate 判定行 20（`pathological_eval_child` 以 loop / microtask 各一次）。（历史口径；现门禁一次 `--ignored` 运行，按 cargo 退出码整体判定。）
- 逐条「为何 ignore / 运行条件 / 判定方式」：`docs/qa/ignored-tests-2026-11-27.md`。

### 性能分布（引用 `docs/qa/perf-distribution-2026-11-27.md`）

- 5000 标题：打开 p95 **96.463 ms**、单步滚动 p95 **1.098 ms**（n=9）。
- 版本历史面板 5000 快照：打开 p95 **10.738 ms**、滚动 p95 **3.642 ms**、关闭 p95 **0.423 ms**（n=9）。
- 本轮 probe-gate 复跑：5000 标题打开 p95 100.234 ms（历史阈值 115.755 ms）、历史面板打开 p95 10.213 ms、滚动 p50 3.615 ms。（阈值口径已随 `85b0953` 删除，现由测试内自证上限。）
- 三次连续稳定性归档：`docs/qa/evidence/assisted-2026-11-27/probe-gate-stability-{1,2,3}.md`（均 PASS 20 / FAIL 0 / SKIP 0）。（归档已随 `85b0953` 删除，仅保留本节历史记录。）
- 完整 min/p50/p95/max 与口径修订说明见 `docs/qa/perf-distribution-2026-11-27.md`，本节不重抄。

### 发布产物与不变量

- `target/release/veil.exe` = **14,309,888 B**（门禁 14,680,064 B，余量 370,176 B）。（已被顶部「直接修复轮」的 14,312,448 B 取代。）
- SHA-256 = `7B5DD8596D95E90CEAEED68AC5FBCDE8CDCB69FAFA83A905C780C76957EDBC6E`。（已被顶部的新 SHA 取代。）
- 未推送 GitHub（本轮统计时 `main` 领先 `origin/main` 6 个 commit）、未打 tag、版本号仍 **3.0.2**。
- 未新增依赖：`Cargo.toml` / `Cargo.lock` 无改动。
- 本轮 commit（新→旧）：`d57be44`、`68846e9`、`66fe135`、`f89bb99`、`370f522`、`906934c`。

## 代码

- 未推送 GitHub、未打 tag、未修改版本号。
- 未新增任何依赖（工作区 Cargo.toml / Cargo.lock 无变化）。

## 测试（2026-11-27 轮次；当前数字见顶部「直接修复轮」一节）

- `cargo test --workspace`：**2258 passed / 0 failed / 17 ignored**（最终轮）。
- 分套件：`veil` lib 286（1 ignored）、`docs_policy` 6、`syntax_matrix` 5（内部 **2148 用例 × 7 表面**）、`veil-core` lib 363（1 ignored）、`conformance` 1、`core` 86、`gfm_conformance` 1、`parser_differential` 2、`real_world_check` 1、`pathological` 0（1 ignored）、`veil-mermaid-offline` 27（2 ignored）、`veil-ui` lib 1471（12 ignored）、`boundary_guard` 7。
- 环境性偶发：`platform::http::tests::actual_loopback_peer_fails_closed` 在早前首次全量运行报 WinHTTP 12152，单独重跑与随后全量复跑均通过；未修改断言、未忽略用例。最终轮未复现。

## 静态检查（2026-11-27 轮次；本轮结果见顶部「最后一公里」一节）

- `cargo clippy --workspace --all-targets -- -D warnings`：0 warnings。
- `cargo fmt --all --check`：通过。

## 发布产物（2026-11-27 轮次；本轮体积与 SHA 见顶部「最后一公里」一节）

- `target/release/veil.exe` = **14,299,648 B**（门禁 14,680,064 B，余量 380,416 B）。
- SHA-256 = `52D82E5F9B905C8BCBB2E51E519B9DD40AEA7925FEC0FF95A38367EC72594FAB`。

## 语法矩阵

- **2148 用例 / 15036 断言 / 0 FAIL / 0 NOT RUN**；`report-2026-11-27.md` 由测试生成。
- 组成：语料 175 条 × 7 = 1225；`ADJB` 306 × 7 = 2142；`ADJI` 288 × 7 = 2016；`ADJC` 90 × 7 = 630；`ADJN`（块级单换行）306 × 7 = 2142；`ADJI2`（换行 + 5 标点边界）864 × 7 = 6048；`ADJC2`（容器内两个内层块）110 × 7 = 770；`ADJX`（嵌套容器）9 × 7 = 63。
- 例外表（规范允许合并）逐条有依据：`ADJB-077/149/257`、`ADJI-002/028/054`；模型边界（非缺陷）：ADJB/ADJN 默认配置无 Callout 块型、ADJC 容器内 Math/嵌套 Admonition 不独立成块。
- 相邻矩阵发现并修复的真实缺陷：`ADJB-086/239`（HTML 导出 defs 前缀把缩进代码降级为段落）、`ADJI-184`（两个相邻高亮的 4 连等号边界被整段降级为字面文本）、`ADJN`（单换行段落 + 分隔行：core 曾整段判为段落，HTML 已是表格；已在 `scan_one` 复刻 cmark-gfm 的「分隔行提升段落末行为表头」）。
- 已登记未修复：`ADJX-003/006`（ListItem / BlockQuote 内的 `<details>` 不被识别为 Details 块，容器展开只提升 Table/Mermaid；断言锁定当前降级形态，修复需扩展容器展开路径）。
- 全量 HTML 摘要 `docs/qa/adjacency-digest.txt`（1973 行：306+288+90+306+864+110+9）逐条重算比对；抽样全文金样 295 份。

## 完美表面收敛（converge-perfect-writing-surface，2026-11-27）

- 事实底座：`docs/qa/perfect-surface-audit-2026-11-27.md`（34 条 F 台账：P0 0 / P1 15 / P2 14 / P3 5；101 个 spec 目录中 27 个历史目录含未勾选，逐目录裁决）。
- 分页一致性：原生 PDF 与打印都只在表格整行边界断页，续页重复表头；单行高于一页才允许行内硬切。单测：PDF `table_paginates_on_row_boundaries_and_repeats_header`；打印 `print_table_page_breaks_land_on_row_tops_and_reserve_header`、`print_repeats_table_header_pixels_on_continuation_pages`。图片/Mermaid 两路径都缩放到一页内；高于一页的显示公式也改为同规则（打印 `print_scales_tall_display_math_to_one_page`、PDF `tall_display_math_scales_to_single_page_content_height`），原差异已闭合（`docs/review/deferred-and-manual-register-2026-11-27.md` §2 保留历史说明）。
- WYSIWYG：窄窗宽表可 `Shift+滚轮` 横向平移读完（`narrow_window_wide_table_readable_via_shift_wheel_pan`，绘制/命中/光标/列宽拖动同口径）；22 类 Mermaid 深色逐图目检并留证据（`docs/qa/evidence/mermaid-dark-2026-11-27/`、`docs/qa/mermaid-dark-2026-11-27.md`），新增 `dark_corpus_renders_differs_and_rasterizes_opaque`；四态降级核对见 `docs/qa/wysiwyg-complex-content-2026-11-27.md`。
- 全维度审计：`docs/review/full-dimension-audit-2026-11-27.md`（架构/十一维度/八原则，P0 = 0，P1 = 6，其中 4 条由本轮任务关闭、2 条为人工证据项）。
- 文档真实性与证据：AccessKit/UIA 已于 `3a6d72d` 移除，中英 README 与组件清单已同步改为「不提供屏幕阅读器支持」；人工验证项与延后项见 `docs/review/deferred-and-manual-register-2026-11-27.md`。
- 门禁：`cargo test --workspace` 2258 passed / 0 failed / 17 ignored；clippy 0 warnings；fmt 通过；`veil.exe` 14,299,648 B（余量 380,416 B）。
- 本轮 6 个 commit（`db65ec8` → `aa39b8c`），未推送、未打 tag、未改版本号、未新增依赖。

## 相邻组合覆盖（cover-adjacent-pairs-and-deferred-items）

- `ADJB`：18 个块单元 × 18 有序对；`ADJI`：12 个行内单元 × 12 × 2 边界；`ADJC`：5 类容器 × 18 块单元。
- **G7 已修复**：列表/引用/Admonition/Callout/Details 内的 ```mermaid 提升为带容器归属的 Mermaid 块，HTML 导出为 `<svg>`，`ADV-011` 同时断言 `<svg>` 与 `<li>` 且无 `language-mermaid`；`exception-states.md` 的延后行已删除。
- 代码块分页：单个代码块高度 ≤ 一页可用高度时整块移页；超长块仍可分页。新增 3 个单元测试；系统打印件复验 6 行代码全在第 3 页。
- PDF 体积：根因是未压缩的 1520×2190 RGB 位图（9,986,400 B），**不是字体**；改为 Flate + PNG predictor 后真实整档 `veil-export-compressed.pdf` 为 **341,076 B**（旧件 10,262,757 B，缩小约 30.1 倍）。
- 打印字体度量差异进入「已接受的环境差异」；Sumatra 第 3 页 Mermaid 已截图；高对比度编辑器/命令面板/删除确认三处已截图并恢复系统 `Flags=126`；5000 标题解析 148.39 ms 登记 ≤200 ms 预算。

## 真机走查

- 浅色 · 所见即所得：240 / 320 / 480 / 800 宽。
- 浅色 · 源码模式（Ctrl+/）：800 宽。
- 深色 · 所见即所得（`VEIL_HOME` 隔离配置，未触碰用户配置）：800 宽。
- 截图证据在临时目录 `%TEMP%\veil-spec-walkthrough\`（不入库）。
- 专注/打字机模式的结构与分页由语法矩阵七表面覆盖；专注模式压暗对比度由单测锁定（被压暗正文 ≥3:1）。

## 设计系统变更摘要

- 浅色语法 7 个 token 全部 ≥4.5:1；深色标点 4.78:1。
- 屏幕 / HTML / PDF 标题字号同源于 `type_scale`；HTML 导出补齐 h1–h6，h5/h6 不小于正文。
- 导出 `mark` 与公式失败配色改由主题派生，深色导出不再出现亮黄块。
- alpha 阶梯单源化；专注/拖放压暗收敛到 40% 档。
- 空态改用中性色；异常四态矩阵 44 格成文，G1/G2/G3/G5 已修；G6 已由 PDF 字阶修复移出，G4 经实测关闭，G7（容器内 Mermaid 不提升）已修复关闭。
- 术语表 18 条；英文标签预算 39 字符；应用内图标不建系统，仅单色语义徽标。

## 渲染与输出收口（close-render-output-gaps）

### W7 高对比度单一来源

- `high_contrast()` 的中性色改由 `theme::shared_palette` 的统一灰阶登记表派生；link/danger/accent 由共享色相锚点提亮至纯黑底 7:1；`mark` 由 mark 色相压暗至白字 7:1；7 个语法 token 由深色色板按 `code_bg` 4.5:1 提亮派生。删除全部手写高饱和第二套色值。
- 真机系统高对比度复验：`SPI_SETHIGHCONTRAST` 置位（Flags `0x77`）→ 启动 app（`mode=system`）→ 截图 → `finally` 恢复（Flags `0x7E`，桌面 `Window=255 255 255`）。截图：`docs/qa/evidence/w10-2026-11-27/w7-high-contrast-system.png`。

### W8 PDF 字阶

- `type_scale` 新增 `PAPER_CODE_RATIO=0.875`、`PAPER_TABLE_RATIO=0.8` 与 `const fn paper_code_size()/paper_table_size()`。
- PDF 代码 **9.625 pt**、表格 **8.8 pt**；`pdf_layout` 已无 9.3/8.8 任意字面量；`syntax_matrix` 页数断言未漂移。

### W9 组合覆盖

- 175 用例 / 1225 断言全部执行，0 FAIL / 0 NOT RUN；金样逐份核对，未发现结构丢失。
- 初始发现并登记 **G7**：列表/引用等容器内的 ```` ```mermaid ```` 不提升为 Mermaid 块，导出保持为容器内代码块，不自动出图（结构未丢失）。本轮已修复（见上「相邻组合覆盖」），`ADV-011` 与 `ADJC` 已断言 `<svg>` 与容器结构共存。

### W10 真实输出

- 证据与逐项结论：`docs/qa/w10-output-pipeline-evidence.md`；产物在 `docs/qa/evidence/w10-2026-11-27/`。
- 深色 HTML：`#1e1e1e` 底、`mark #5f4c09`（非亮黄）、h1×1/h2×7、表格/代码/公式/Mermaid 正常，front matter 未泄漏。
- 原生 PDF：修复前历史件 `veil-export.pdf` 为 4 页、10,262,757 B；修复后 `veil-export-compressed.pdf` 为 5 页、341,076 B（代码块整块后移导致页数变化），`msyh`/`consola` 可搜索；页边距 20 mm。
- Microsoft Print to PDF：5 页、402,145 B；页边距/分页与原生导出一致；代码块 6 行全部落在第 3 页不跨页；驱动字体替换导致行内代码后标点极小间距差异（已记录）。
- PDF 在 Edge：目录页与第 3 页（Mermaid）截图通过；SumatraPDF 3.5.2 便携版打开原生 PDF，目录/书签/CJK 正常，第 3 页 Mermaid 已单独截图（`pdf-sumatra-page3-mermaid.png`）；MuPDF 1.28.6 对两份 PDF 逐页渲染与文本提取均通过。
- 原生 PDF 体积偏大的根因已更正：是未压缩的 1520×2190 RGB 位图（9,986,400 B，占 97.3%），**不是** CJK 字体子集；本轮以 Flate + PNG predictor 修复，整档缩至 341,076 B。

### W11 G4 裁决

- 5000 文件：快速打开合计约 **13 ms**（release，20 次中位）；5000 标题大纲块枚举 **0.34 ms**；均远低于 100 ms，**G4 关闭**，不引入异步加载态。
- 附注：5000 标题文档 `Session::open` 约 148 ms，属文档打开路径，不属于 G4；已登记 **≤200 ms 预算**，复测方法见 `docs/qa/w11-load-state-measurement.md`。本轮不改解析器。
