# spec `polish-veil-ux-craft` 最终量化复验 · final-v2（最终产物重建后重跑）

- 结论：**PASS**（启动 p95 609.02 ms ≤ 1000；首帧黑像素 0%；20k 击键 p95 2.856 ms ≤ 16；8 表面最低单项 4.5 ≥ 4.5）
- 日期：2026-11-21（Asia/Shanghai）
- 本轮全部重新测量，未沿用 `evidence/ux/final/` 的任何数字；旧目录保持原样未覆盖

## 1. 产物校验

| 项目 | 值 |
|---|---|
| 路径 | `target/release/veil.exe` |
| 字节数（开始/结束） | 14,010,368 / 14,010,368 |
| SHA-256（开始/结束） | `17D8E0813FF4197E58FD2335867A6118C10941772342B85334650183BECA5EE1`（一致） |
| commit | `345d8c6` |
| 记录 | `evidence/ux/final-v2/exe-verify-{start,end}.txt` |

环境：Windows，物理 2560×1440 / 逻辑 1707×960，DPI **144（150%）**；六份采集日志均记录 `真实窗口 DPI=144（比例=1.5）`。未修改 tracked 文件，未 commit/push。

## 2. 冷启动 5 次（隔离 VEIL_HOME/APPDATA/TEMP/文档）

| run | start_to_window_ms | 首帧黑像素（0/30/100/300/800 ms） | 正常关窗 | 临时根清理 | exit |
|---|---:|---|---|---|---:|
| 01 | 609.50 | 0/0/0/0/0（0 / 1,417,500 px） | True | True | 0 |
| 02 | 585.15 | 0/0/0/0/0 | True | True | 0 |
| 03 | 599.71 | 0/0/0/0/0 | True | True | 0 |
| 04 | 582.27 | 0/0/0/0/0 | True | True | 0 |
| 05 | 607.12 | 0/0/0/0/0 | True | True | 0 |

- **p50 = 599.71 ms，p95 = 609.02 ms，max = 609.50 ms**（mean 596.75）。
- p95 ≤ 1000 ms：**PASS**；首帧黑像素 < 0.1%：**PASS（五轮全 0%）**。
- 较上一版（p50 531.78 / p95 544.76）慢约 12%，但仍在门槛内。
- 证据：`evidence/ux/final-v2/startup/run-01…05/dpi-150/`。

## 3. 真实窗口输入

退出码 0，`status=PASS`：键入 38 B（`5464B4F1…45B0`）、撤销 30 B（`54AB3751…2212`）、导出前 40 B（`CDE05421…5C`）、`Ctrl+E` HTML 导出 2,085 B（断言 `doctype+base+VEILEXPORT`，`CB44D90C…27E`）、正常关窗 ExitCode=0。文档为临时目录独立副本，未触碰真实用户文档。证据：`evidence/ux/final-v2/input/input/`。

## 4. 20k 击键（release，两轮独立执行）

| run | samples | p50 | p95 | max | exit |
|---|---:|---:|---:|---:|---:|
| 1 | 300 | 2.395 ms | **2.856 ms** | 3.184 ms | 0 |
| 2 | 300 | 2.329 ms | **2.696 ms** | 3.107 ms | 0 |

p95 ≤ 16 ms：**PASS**。日志：`evidence/ux/final-v2/perf/run{1,2}.log`。

## 5. 150% 八表面 rubric（重新截图、逐张目检，共 104 张 PNG）

采集：隔离 `VEIL_HOME`（`config.toml: mode`）+ `APPDATA`/`TEMP`，临时工作目录放置 `corpus.md`/`veil-logo.png`；应用内滚动覆盖全部 Mermaid 后经 GUI + 原生对话框导出 HTML/PDF；导出件在独立 Edge profile 中按 150% DPI 打开逐屏截图。应用 1422×1106、HTML 1422×1338、PDF 1222×1338 物理像素。应用退出码 0 并以 `CloseMainWindow` 正常关闭；查看器退出码 0；日志无 PrintWindow 降级。

| 表面 | 内容 | 顺序 | 裁切 | 重叠/遮挡 | 对比度 | 字号 | 表格 | 图片 | 公式 | Mermaid | 均值 | 最低 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| light/编辑态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| light/源码态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| light/HTML | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| light/PDF | 5 | 5 | 5 | 5 | 5 | **4.5** | 5 | 5 | 5 | 5 | 4.95 | 4.5 |
| dark/编辑态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| dark/源码态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| dark/HTML | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 |
| dark/PDF | 5 | 5 | 5 | 5 | 5 | **4.5** | 5 | 5 | 5 | 5 | 4.95 | 4.5 |

**8 表面最低单项 = 4.5（PDF 字号），无 <4.5、无均值抵消、无回归，判定 PASS。**

证据文件（每目录含 `walkthrough.log` 与逐屏 PNG）：

- light 编辑：`visual/light/app/02-step-edit-00-20261121-141342.png … 17-step-edit-bottom-…`（07 之后为同一末尾视图）
- light 源码：`visual/light/app/19-step-source-00-…` … `28-step-source-bottom-…`
- light HTML：`visual/light/html/02-step-html-00-20261121-141914.png … 13-step-html-bottom-…`
- light PDF：`visual/light/pdf/02-step-pdf-00-20261121-141946.png … 09-step-pdf-bottom-…`（4 页）
- dark 编辑/源码：`visual/dark/app/02-step-edit-00-20261121-141610.png …`、`19-step-source-00-…`
- dark HTML/PDF：`visual/dark/html/02-step-html-00-20261121-142014.png …`、`visual/dark/pdf/02-step-pdf-00-20261121-142046.png …`

逐项核对要点：三列表格完整且 `code` 未断行；图片 `VEIL` 正常加载（本轮“图片错误文案变化”未影响已存在图片）；行内 `E=mc²` 与块公式根号/分数线完整；流程图“是/否”标签与箭头无文字遮挡；时序图、甘特图、思维导图（根节点 `Veil` 文字完整）均完整；源码态代码块首尾围栏完整；8 表面 `FINAL-SENTINEL` 均可见。

## 6. 导出件与 `evidence/render/task9-final/` 对比

| 主题/类型 | 字节 | 本轮 SHA-256 | 基线 SHA-256 | 结论 |
|---|---:|---|---|---|
| light HTML | 74,703 | `5EEC129600FA17BD839352254BBA474066AB830F0DE5B71D9D578604A87B3085` | `F8361A0703BFA9B897913D6612084F02E4125149BABE1C953B3401C1BBE8CC01` | 仅 today 线 4 字节 |
| light PDF | 27,774,154 | `8120795C45632B75373AEBCCBD22BE740E34C1E5EA9C9140A946A0467725D1BB` | 同左 | **字节级一致** |
| dark HTML | 78,577 | `5AE2C975A17F95A41A25DD9E83CF29E9356883EC9622D365AAF43B08D2D6EF8B` | `AE94D0830D19CB173E2AB28AC810DCF0D111C02D63A1F368201C8E079CA070B1` | 仅 today 线 4 字节 |
| dark PDF | 27,774,154 | `8120795C…D1BB` | 同左 | **字节级一致** |

HTML 逐字节实测：light 差异偏移 `53288,53289,53299,53300`；dark 为 `56860,56861,56871,56872`；均为 `06 甘特图` 的 `<g class="today"><line class="today" … x1/x2 />` 由基线 **21160** 变为 **21174**（右移 **14 px**），其余字节相同；符合 today 线随导出时刻移动的允许项，截图层面除该竖线位置外无可视差异。PDF 差异 0 字节（含目录页与 4 页分页）。

## 7. 非阻断观察

- PDF 第 2 页末尾孤立标题 `04 Mermaid 流程图`、第 3 页末尾孤立标题 `07 Mermaid 思维导图`，图体在下一页且页底留白较大；因导出件与 task9-final 字节级一致，属既有分页现象，无裁切/顺序错误，未据此扣分。
- dark 主题 PDF 使用浅色打印图（与基线一致）；编辑态活动块源码 reveal 属既定行为。

## 8. needs-user（NOT RUN）

- 真实 100% DPI 的 light/dark × 编辑/源码/HTML/PDF：**needs-user / NOT RUN**，未由 150% 推断。
- 物理打印：**needs-user / NOT RUN**，未预填 PASS。

## 9. 产物

- `evidence/ux/final-v2/summary.json`（含 224 个证据文件字节数与 SHA-256、全部数字与阈值判定）。
- `evidence/ux/final-v2/report.md`（本文件）。
- 可复现脚本：`evidence/ux/final-v2/_scripts/{visual.ps1,edge-capture.ps1,write-summary.ps1}`。
- 结束复核：exe 字节/SHA 与开始时一致；无残留 Veil/Edge 进程；临时目录已清理。
