# W10 真实输出链路证据（2026-11-27）

本文件只记录**实际跑过并留下产物**的结论；未执行的项目明确写“未验证”，不替代为推测。

## 环境

| 项 | 值 |
|---|---|
| 系统 | Windows（150% 缩放，宿主屏幕逻辑尺寸较小） |
| 可执行文件 | `target\release\veil.exe`，14,277,120 B，SHA256 `61B1A6426C57B1BDE6D6C06F0649DE2ABFE34666407187ADA5CC9BC9648C4262` |
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

产物：`print-to-pdf.pdf`（401,538 B，4 页），逐页渲染 `print-pdf-mupdf-p1..4.png`。

| 检查项 | 结论 |
|---|---|
| 页边距 | 第 2 页文字包围盒左边界 **56.8 pt ≈ 20.0 mm**；与原生导出的包围盒逐值一致 |
| 分页 | 4 页：第 1 页目录，第 2 页标题/正文/引用/列表/表格/代码起，第 3 页代码续/公式/Mermaid，第 4 页压力段与结尾 |
| 中文断行 | 汉字间自然断行，未见行首禁则字符；超长 URL 在内容框内断开 |
| 表格 | 网格、表头、单元格内换行完整，无越界 |
| 代码块 | 跨页时分为页 2 底与页 3 顶两段，**内容完整**；底色与等宽字体保留 |
| 字体 | 驱动输出为 `CIDFont+F1/F2`（Type0/Identity-H）；文本可提取（第 2 页 1059 字符，其中 CJK 403），无缺字 |
| 元数据 | `/Producer (Microsoft: Print To PDF)`，PDF 1.7 |

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
- **SumatraPDF**：本机原无 SumatraPDF，本轮下载便携版 **3.5.2 (64-bit)** 实测。打开原生 PDF 后目录、书签侧栏与中文字形均正常，页脚 `1 / 4` 正确（`pdf-sumatra-toc.png`）。未能在该版本中自动跳转到第 3 页，因此 **Mermaid 页未在 Sumatra 单独截图**，该页由 Edge 与 MuPDF 覆盖；不据此声称“Sumatra 全页核验完成”。
- **MuPDF**：对原生导出与打印输出两份 PDF 各渲染 4 页并提取文本：
  - 原生导出字体 `msyh` / `consola`（Type0/CIDFontType2 + Identity-H），文本可提取；
  - 打印输出字体 `CIDFont+F1/F2`，文本可提取；
  - 复现命令：`python docs/qa/evidence/w10-2026-11-27/render_pdf.py`（原生导出）与 `python docs/qa/evidence/w10-2026-11-27/render_print_pdf.py`（打印输出），脚本随证据一同保留。
  - 前置条件仅为安装 PyMuPDF（`pip install pymupdf`）；本机已具备，未要求额外系统组件。

## 原生 PDF 导出的两个事实

1. **版面几何**：4 页；第 2 页文字包围盒 `(56.7, 65.8) – (534.6, 815.9)`，左/右页边距 20 mm；目录页与正文页均存在；页脚 `n / total`。
2. **体积**：4 页 **10,262,757 B**。原因是嵌入了 CJK 正文字体（`msyh`）与等宽字体（`consola`）的子集；功能与可搜索性正常，但体积明显大于打印输出的 401 KB。**本轮不修**，如实记录为后续优化项，不声称“体积已优化”。

## 结论

| 链路 | 结论 | 证据 |
|---|---|---|
| 深色 HTML 导出 + Edge | 通过 | `veil-dark.html`、`dark-html-edge-full.png` |
| 原生 PDF 导出 | 通过（体积偏大，已记录） | `veil-export.pdf`、`pdf-mupdf-p1..4.png` |
| Microsoft Print to PDF | 通过（存在驱动字体度量差异，已记录） | `print-to-pdf.pdf`、`print-dialog.png`、`print-pdf-mupdf-p1..4.png` |
| PDF 在 Edge | 通过 | `pdf-edge-toc.png`、`pdf-edge-page3-mermaid.png` |
| PDF 在 Sumatra | 通过（目录/CJK/书签；Mermaid 页未单独截图） | `pdf-sumatra-toc.png` |
| PDF 在 MuPDF | 通过 | 两份 PDF 的 4 页渲染与文本提取 |
