# Veil 交付事实（2026-11-27）

> 本页为人工维护的交付事实。`report-2026-11-27.md` 由语法矩阵测试生成，运行测试会覆写，请勿手工追加。

## 代码

- 未推送 GitHub、未打 tag、未修改版本号。
- 未新增任何依赖（工作区 Cargo.toml / Cargo.lock 无变化）。

## 测试

- `cargo test --workspace`：**2251 passed / 0 failed / 17 ignored**（最终轮）。
- 分套件：`veil` lib 285（1 ignored）、`docs_policy` 6、`syntax_matrix` 5（内部 **859 用例 × 7 表面**）、`veil-core` lib 363（1 ignored）、`conformance` 1、`core` 86、`gfm_conformance` 1、`parser_differential` 2、`real_world_check` 1、`pathological` 0（1 ignored）、`veil-mermaid-offline` 26（2 ignored）、`veil-ui` lib 1466（12 ignored）、`boundary_guard` 7。
- 环境性偶发：`platform::http::tests::actual_loopback_peer_fails_closed` 在早前首次全量运行报 WinHTTP 12152，单独重跑与随后全量复跑均通过；未修改断言、未忽略用例。最终轮未复现。

## 静态检查

- `cargo clippy --workspace --all-targets -- -D warnings`：0 warnings。
- `cargo fmt --all --check`：通过。

## 发布产物

- `target/release/veil.exe` = **14,291,456 B**（门禁 14,680,064 B，余量 388,608 B）。
- SHA-256 = `0C03391B4EF8B92605D85AC8DD0A4CB514EE1F2A4F2A560AEFAD4CA37B33FAC8`。

## 语法矩阵

- **859 用例 / 6013 断言 / 0 FAIL / 0 NOT RUN**；`report-2026-11-27.md` 由测试生成。
- 组成：语料 175 条 × 7 = 1225；生成式块级相邻 `ADJB` 306 条 × 7 = 2142（18 个 B=FrontMatter 对按理由 N/A）；行内相邻 `ADJI` 288 条 × 7 = 2016；容器内一级内容 `ADJC` 90 条 × 7 = 630。
- 例外表（规范允许合并）逐条有依据：`ADJB-077/149/257`、`ADJI-002/028/054`；模型边界（非缺陷）：ADJB 默认配置无 Callout 块型、ADJC 容器内 Math/嵌套 Admonition 不独立成块。
- 相邻矩阵发现并修复的真实缺陷：`ADJB-086/239`（HTML 导出 defs 前缀把缩进代码降级为段落）、`ADJI-184`（两个相邻高亮的 4 连等号边界被整段降级为字面文本）。修复后结构断言全量执行，无「暂缓/待修」。
- 全量 HTML 摘要 `docs/qa/adjacency-digest.txt`（684 行）逐条重算比对；抽样全文金样 226 份。

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
