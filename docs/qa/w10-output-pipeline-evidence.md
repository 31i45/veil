# W10 真实输出链路证据（2026-11-27）

本文件只记录**实际跑过并留下产物**的结论；未执行的项目明确写“未验证”，不替代为推测。

## 环境

| 项 | 值 |
|---|---|
| 系统 | Windows（150% 缩放，宿主屏幕逻辑尺寸较小） |
| 可执行文件 | `target\release\veil.exe`，14,291,456 B，SHA256 `0C03391B4EF8B92605D85AC8DD0A4CB514EE1F2A4F2A560AEFAD4CA37B33FAC8`（本轮最终构建） |
| 隔离配置 | `VEIL_HOME=%TEMP%\veil-w10\home*`，`config.toml` 仅写 `mode = "dark"` 或 `mode = "system"` |
| 浏览器 | Microsoft Edge 158.0.4370.33 |
| PDF 驱动 | Microsoft Print to PDF（端口 `PORTPROMPT:`，系统默认打印机即此机） |
| PDF 渲染/解析 | PyMuPDF 2.1（MuPDF 1.28.6） |

## 样张与操作路径

- 样张：`docs/qa/evidence/w10-2026-11-27/sample.md`（front matter、H1–H2、CJK+Latin 混排、超长 URL、引用、列表、表格、Rust 代码、行内/行间公式、Mermaid、3 段分页压力段）。
- HTML/PDF 导出：真实 app 中按 `Ctrl+E` / `Ctrl+Shift+E`，在原生保存对话框输入证据目录路径。
- 系统打印：真实 app 中按 `Ctrl+Alt+P`，现代打印对话框选择 **Microsoft Print to PDF**（证据 `print-dialog.png`），保存到 `print-to-pdf.pdf`（证据 `print-save-dialog.png`）。
- 说明：现代打印对话框不是 Veil 进程拥有的 `#32770`，`tools/walkthrough.ps1` 的 `wait-dialog` 无法识别；改用 UI Automation 找到“打印”按钮并调用，随后由系统保存对话框写盘。**没有**改用物理打印机（Pantum）出纸。

## 4.1 Microsoft Print to PDF

产物：`print-to-pdf.pdf`（**402,145 B，5 页**，本轮用最终 exe 重打），逐页渲染 `print-pdf-mupdf-p1..5.png`。

| 检查项 | 结论 |
|---|---|
| 页边距 | 第 2 页文字包围盒左边界 **56.8 pt ≈ 20.0 mm**；与原生导出的包围盒逐值一致 |
| 分页 | 5 页：第 1 页目录，第 2 页标题/正文/引用/列表/表格，第 3 页**完整代码块**/公式/Mermaid，第 4 页压力段，第 5 页结尾；无空页 |
| 中文断行 | 汉字间自然断行，未见行首禁则字符；超长 URL 在内容框内断开 |
| 表格 | 网格、表头、单元格内换行完整，无越界 |
| 代码块 | **不再跨页**：6 行代码（`fn main` 到闭合 `}`）全部在第 3 页，文本提取逐行命中同一页；因整块后移，总页数由旧件 4 页变为 5 页，第 5 页承载「七、结尾」，不是空白页 |
| 字体 | 驱动输出为 `CIDFont+F1/F2`（Type0/Identity-H）；文本可提取，无缺字 |
| 元数据 | `/Producer (Microsoft: Print To PDF)`，PDF 1.7 |

复验命令：`python tools/verify/verify_print_codeblock.py`（逐页提取文本，断言 6 行代码同页、无缺行/重复）。

**如实差异**：打印链路的行内代码后标点间距与原生导出的 PDF 有极小差别（例：`cargo test --workspace 。` 比原生多一个视觉空隙），来自打印机驱动的字体替换度量；未造成断行、越界或内容丢失。

## 4.2 深色 HTML 在 Edge

产物：`veil-dark.html`（31,428 B）、`dark-html-edge-full.png`（整页，含 Mermaid）。

| 检查项 | 结论 |
|---|---|
| 底色/前景 | `#1e1e1e` / `#dcdcdc`，确为深色导出 |
| mark | `#5f4c09`（暗琥珀），**不是**亮黄色块 |
| 标题层级 | `<h1>` ×1、`<h2>` ×7，与源文一一对应 |
| 结构 | `<table>` ×1、`<pre><code>` ×1、`<svg>` ×3（Mermaid ×1 + 公式 ×2） |
| front matter | `title: W10` 等元数据未泄漏到正文 |
| Edge 渲染 | 标题、表格、代码、公式、Mermaid 全部正常；长 URL 在栏内换行 |

## 4.3 PDF 在 Edge / Sumatra / MuPDF

- **Edge**：打开原生导出的 PDF，目录页与第 3 页（含 Mermaid）截图见 `pdf-edge-toc.png`、`pdf-edge-page3-mermaid.png`；4 页均可浏览。
- **SumatraPDF**：本机原无 SumatraPDF，本轮下载便携版 **3.5.2 (64-bit)** 实测。打开原生 PDF 后目录、书签侧栏与中文字形均正常，页脚 `1 / 4` 正确（`pdf-sumatra-toc.png`）。本轮通过 UI 操作（Ctrl+G 输入 3）补拍第 3 页：页脚显示 `3 / 4`，画面含 Mermaid 流程图（`pdf-sumatra-page3-mermaid.png`）。该页取自修复前原生 PDF，用于视觉核验；脚本化失败原因（`-nosession` 等选项被当成文件、PrintWindow 抓白、后台窗口遮挡）已记录，最终改用真实 UI 操作完成。
- **MuPDF**：对原生导出与打印输出两份 PDF 各渲染 4 页并提取文本：
  - 原生导出字体 `msyh` / `consola`（Type0/CIDFontType2 + Identity-H），文本可提取；
  - 打印输出字体 `CIDFont+F1/F2`，文本可提取；
  - 复现命令：`python docs/qa/evidence/w10-2026-11-27/render_pdf.py`（原生导出）与 `python docs/qa/evidence/w10-2026-11-27/render_print_pdf.py`（打印输出），脚本随证据一同保留。
  - 前置条件仅为安装 PyMuPDF（`pip install pymupdf`）；本机已具备，未要求额外系统组件。

## 原生 PDF 导出的两个事实

> 本节第 1 条描述**修复前历史件** `veil-export.pdf`（4 页）；修复后整档为 `veil-export-compressed.pdf`（5 页，代码块整块后移导致页数变化），见上。

1. **版面几何（历史件）**：4 页；第 2 页文字包围盒 `(56.7, 65.8) – (534.6, 815.9)`，左/右页边距 20 mm；目录页与正文页均存在；页脚 `n / total`。
2. **体积（已更正）**：4 页 **10,262,757 B** 的主导项**不是字体子集**，而是一张 **1520×2190、DeviceRGB、完全未压缩**的图像 XObject：流长度 `1520×2190×3 = 9,986,400 B`，占整档 **97.3%**。
   - 复核方法：按 PDF 对象解析该档的 `/Subtype /Image`，得到 `W=1520 H=2190 CS=DeviceRGB BPC=8 Filter=（无）Len=9986400`；另两张小图分别为 4,503 B（79×19）与 31,752 B（147×72）。
   - 字体实际很小：`/FontFile2` 流仅 173,292 B（主 CJK 子集）与 31,592 B（回退/等宽子集），ToUnicode CMap 亦很小。此前“10.26 MB 主要来自 CJK/等宽字体子集”的结论**错误**，现予更正。
   - 该位图来自 Mermaid 图在 PDF 路径上的 300 dpi 光栅化（`pdf_layout::mermaid_block` → `svg_to_bitmap(300/96*2)` → `ImageData` RGB8）；原实现直接把原始 RGB 写入 PDF Image XObject，未做任何压缩。
   - **修复后整档复测**：用最终 exe 对同一样张真实导出为 `veil-export-compressed.pdf`（**341,076 B，5 页，3 张图片，8 条书签**）。对象解析：最大图像流 **64,214 B（FlateDecode）**，另两张 4,503 B / 31,752 B；整档比修复前 10,262,757 B **缩小约 30.1 倍**，低于 1 MiB 预算。页数由 4 变 5 与打印链路一致，原因是代码块整块后移（同一版面模型），不是压缩导致。逐页渲染见 `compressed-pdf-p1..5.png`（第 3 页含 Mermaid 图）。

## Task 6.1 修复：图像 Flate 压缩（本轮）

- 代码位置：`crates/veil/src/platform/pdf.rs`，`write_image_xobject` / `compress_image_samples` / `png_idat_stream`。
- 策略：原始 RGB 样本 ≥ 64 KiB 时，用已有 `png` crate 以 RGB8、`Filter::NoFilter` 编码到内存 PNG，拼接全部 IDAT 数据得到单一 zlib 流；以 `/Filter /FlateDecode` 写入，并设 `/DecodeParms << /Predictor 15 /Colors 3 /BitsPerComponent 8 /Columns <宽> >>`。小图或任何编码失败回退原始写入，导出不会因此失败。未改版面尺寸、DPI、字体子集策略、页数/页码。
- 本轮可复现测量（合成 1520×2190、Mermaid-like 近平坦 RGB 图：渐变底 + 少量色块）：

  | 项 | 数值 |
  |---|---:|
  | 原始 RGB 样本 | 9,986,400 B |
  | PNG/IDAT 压缩后 zlib 流 | 80,765 B（约原始 0.81%，< 1/10） |
  | 含该大图的合成 PDF（单页，含字体子集） | 106,318 B |
  | 旧 `veil-export.pdf`（修复前历史件） | 10,262,757 B |

- 复现命令：`cargo test -p veil --lib platform::pdf::tests`。新增测试：`large_flat_rgb_compresses_under_one_tenth_and_roundtrips`、`noisy_rgb_compresses_losslessly_without_panic`、`small_image_is_left_uncompressed`、`pdf_image_xobject_uses_flate_and_png_predictor`、`render_pages_compresses_large_mermaid_like_image`。表中数值由对应测试临时打印测得；测试断言阈值为「压缩后 < 原始 1/10」与「合成 PDF < 1 MiB」，并要求 PNG 解码逐字节还原原始 RGB。
- 说明：`docs/qa/evidence/w10-2026-11-27/veil-export.pdf` 是**修复前历史件**（10,262,757 B），本轮不覆盖它；修复后整档为 `veil-export-compressed.pdf`（341,076 B），两者并存以便对照。真实整档复测已完成（见上），不再保留“待补做”表述。

## 结论

| 链路 | 结论 | 证据 |
|---|---|---|
| 深色 HTML 导出 + Edge | 通过 | `veil-dark.html`、`dark-html-edge-full.png` |
| 原生 PDF 导出 | 通过（修复后 341,076 B；旧件 10,262,757 B 保留为历史件） | `veil-export-compressed.pdf`、`veil-export.pdf`、`pdf-mupdf-p1..4.png` |
| Microsoft Print to PDF | 通过（代码块不跨页；存在驱动字体度量差异，已记录） | `print-to-pdf.pdf`、`print-dialog.png`、`print-pdf-mupdf-p1..5.png` |
| PDF 在 Edge | 通过 | `pdf-edge-toc.png`、`pdf-edge-page3-mermaid.png` |
| PDF 在 Sumatra | 通过（目录/CJK/书签；第 3 页 Mermaid 已截图） | `pdf-sumatra-toc.png`、`pdf-sumatra-page3-mermaid.png` |
| PDF 在 MuPDF | 通过 | 两份 PDF 的 4 页渲染与文本提取 |
