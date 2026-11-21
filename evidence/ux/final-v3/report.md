# spec `polish-veil-ux-craft` 最终量化复验 · final-v3（最终产物最终版）

- 结论：**PASS**（启动 p95 552.64 ms ≤ 1000；首帧黑像素 0%；20k 击键 p95 3.031 ms ≤ 16；8 表面最低单项 4.5 ≥ 4.5）
- 日期：2026-11-21（Asia/Shanghai）
- 全部重新测量，未沿用 final/ 或 final-v2/ 的数值；旧目录原样保留

## 1. 产物校验

| 项目 | 值 |
|---|---|
| 路径 | `target/release/veil.exe` |
| 字节数（开始/结束） | 14,010,880 / 14,010,880 |
| SHA-256（开始/结束） | `FC6BCCD4A11C0214E07974F6227F02BF2C10063E98CE6917B1E6303F14F9D01C`（一致） |
| commit | `ce038eb` |
| 记录 | `evidence/ux/final-v3/exe-verify-{start,end}.txt` |

环境：Windows，物理 2560×1440 / 逻辑 1707×960，DPI **144（150%）**；六份采集日志均记录 `真实窗口 DPI=144（比例=1.5）`。未修改 tracked 文件，未 commit/push。

## 2. 冷启动 5 次（隔离 VEIL_HOME/APPDATA/TEMP/文档）

| run | start_to_window_ms | 首帧黑像素（0/30/100/300/800 ms） | 正常关窗 | 临时根清理 | exit |
|---|---:|---|---|---|---:|
| 01 | 556.33 | 0/0/0/0/0（0 / 1,417,500 px） | True | True | 0 |
| 02 | 526.89 | 0/0/0/0/0 | True | True | 0 |
| 03 | 511.89 | 0/0/0/0/0 | True | True | 0 |
| 04 | 537.86 | 0/0/0/0/0 | True | True | 0 |
| 05 | 531.29 | 0/0/0/0/0 | True | True | 0 |

- **p50 = 531.29 ms，p95 = 552.64 ms，max = 556.33 ms**（mean 532.85）。
- p95 ≤ 1000 ms：**PASS**；首帧黑像素 < 0.1%：**PASS（五轮全 0%）**。
- 证据：`evidence/ux/final-v3/startup/run-01…05/dpi-150/`。

## 3. 真实窗口输入

退出码 0，`status=PASS`：键入 38 B（`5464B4F1…45B0`）、撤销 30 B（`54AB3751…2212`）、导出前 40 B（`CDE05421…5C`）、`Ctrl+E` HTML 导出 2,085 B（断言 `doctype+base+VEILEXPORT`，`CB44D90C…27E`）、正常关窗。文档为临时目录独立副本，未触碰真实用户文档。证据：`evidence/ux/final-v3/input/input/`。

## 4. 20k 击键（release，两轮独立执行）

| run | samples | p50 | p95 | max | exit |
|---|---:|---:|---:|---:|---:|
| 1 | 300 | 2.292 ms | **2.646 ms** | 2.843 ms | 0 |
| 2 | 300 | 2.631 ms | **3.031 ms** | 3.946 ms | 0 |

p95 ≤ 16 ms：**PASS**。日志：`evidence/ux/final-v3/perf/run{1,2}.log`。

## 5. 150% 八表面 rubric（重截、逐张目检，共 104 张 PNG）

采集：隔离 `VEIL_HOME`（`config.toml: mode`）+ `APPDATA`/`TEMP`，临时目录放置 `corpus.md`/`veil-logo.png`；滚动覆盖全部 Mermaid 后经 GUI + 原生对话框导出 HTML/PDF；导出件在独立 Edge profile 按 150% DPI 打开逐屏截图。应用 1422×1106、HTML 1422×1338、PDF 1222×1338 物理像素；应用退出码 0 并 `CloseMainWindow` 正常关闭；查看器退出码 0，日志无 PrintWindow 降级。

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

**8 表面最低单项 = 4.5（PDF 字号），无 <4.5、未用均值抵消、无回归 → PASS。**

证据（每目录含 `walkthrough.log` 与逐屏 PNG）：

- light 编辑：`visual/light/app/02-step-edit-00-20261121-174944.png … 17-step-edit-bottom-…`（07 之后为同一末尾视图）
- light 源码：`visual/light/app/19-step-source-00-…` … `28-step-source-bottom-…`
- light HTML：`visual/light/html/02-step-html-00-20261121-175211.png … 13-step-html-bottom-…`
- light PDF：`visual/light/pdf/02-step-pdf-00-20261121-175243.png … 09-step-pdf-bottom-…`（4 页）
- dark 编辑/源码：`visual/dark/app/02-step-edit-00-20261121-175313.png …`、`19-step-source-00-…`
- dark HTML/PDF：`visual/dark/html/02-step-html-00-20261121-175539.png …`、`visual/dark/pdf/02-step-pdf-00-20261121-175611.png …`

要点：三列表格完整、`code` 未断行；图片 `VEIL` 正常加载；行内 `E=mc²` 与块公式完整；流程图边标签无文字遮挡；时序图/甘特图/思维导图（根节点 `Veil`）完整；源码态代码块首尾围栏完整；8 表面 `FINAL-SENTINEL` 均可见。

## 6. 导出件与 `evidence/render/task9-final/` 对比

| 主题/类型 | 字节 | 本轮 SHA-256 | 基线 SHA-256 | 结论 |
|---|---:|---|---|---|
| light HTML | 74,703 | `E316073213D37420A9A554C5C417F2FA964EDFB9BE811344EC954EFD1F95A88D` | `F8361A0703BFA9B897913D6612084F02E4125149BABE1C953B3401C1BBE8CC01` | 仅 today 线 4 字节 |
| light PDF | 27,774,154 | `8120795C45632B75373AEBCCBD22BE740E34C1E5EA9C9140A946A0467725D1BB` | 同左 | **字节级一致** |
| dark HTML | 78,577 | `F879E6D9AA9DF36D1F4E485178F1C7C64C4083330915E06AC19B2406B6DF9484` | `AE94D0830D19CB173E2AB28AC810DCF0D111C02D63A1F368201C8E079CA070B1` | 仅 today 线 4 字节 |
| dark PDF | 27,774,154 | `8120795C…D1BB` | 同左 | **字节级一致** |

HTML 逐字节实测：light 差异偏移 `53288,53289,53299,53300`；dark 为 `56860,56861,56871,56872`；均为 `06 甘特图` 的 `<g class="today"><line class="today" … x1/x2 />` 由基线 **21160** 变为 **21183**（右移 **23 px**），其余字节完全相同；符合 today 线随导出时刻移动的允许项，截图层面除该竖线位置外无可视差异。PDF 差异 0 字节（含目录页与 4 页分页）。

## 7. 非阻断观察

- PDF 第 2 页末尾孤立标题 `04 Mermaid 流程图`、第 3 页末尾孤立标题 `07 Mermaid 思维导图`，图体在下一页、页底留白较大；导出件与 task9-final 字节级一致，属既有分页现象，无裁切/顺序错误，未据此扣分。
- dark 主题 PDF 使用浅色打印图（与基线一致）；编辑态活动块源码 reveal 属既定行为。

## 8. needs-user（NOT RUN）

- 真实 100% DPI 的 light/dark × 编辑/源码/HTML/PDF：**needs-user / NOT RUN**，未由 150% 推断。
- 物理打印：**needs-user / NOT RUN**，未预填 PASS。

## 9. 产物

- `evidence/ux/final-v3/summary.json`（220 个证据文件字节数与 SHA-256、全部数字与阈值判定）。
- `evidence/ux/final-v3/report.md`（本文件）。
- 可复现脚本：`evidence/ux/final-v3/_scripts/{visual.ps1,edge-capture.ps1,write-summary.ps1}`。
- 结束复核：exe 字节/SHA 与开始时一致；无残留 Veil/Edge 进程；本轮临时目录已清理。
