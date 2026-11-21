# final-v4：最终产物全量重测报告

- 规格：`polish-veil-ux-craft`；日期 2026-11-21；本轮为**重新测量**，未沿用 final-v3 数值。
- 产物：`target/release/veil.exe`，**14,011,392 B**，SHA-256
  `B921300F9795244C66B5BDC4E977AFD0653B947E0761C5269E6D0CEDF237DE3B`，commit **c19a082**。
  开始 19:20 与结束 19:41:53 两次校验一致（`exe-verify-start.txt` / `exe-verify-end.txt`）。
- 环境：Windows，DPI **144（150%）**，物理屏 2560×1440，逻辑 1707×960；所有表面日志 DPI 均为 144。
- **结论：PASS**（性能、首帧、输入、八表面评分全部达标；HTML 仅「今天线」按时间平移，PDF 与基线逐字节一致）。

## 1. 冷启动（5 次，独立 VEIL_HOME/APPDATA/TEMP/文档）

| run | start_to_window_ms | 首帧黑像素峰值 | 800ms 黑像素 | 状态 |
|---|---|---|---|---|
| 01 | 355.82 | 0/2,719,350 (0%) | 0 | PASS |
| 02 | 339.78 | 0 | 0 | PASS |
| 03 | 362.09 | 0 | 0 | PASS |
| 04 | 355.52 | 0 | 0 | PASS |
| 05 | 339.98 | 0 | 0 | PASS |

- **p50 = 355.52 ms，p95 = 360.84 ms，max = 362.09 ms**，均值 350.64 ms；阈值 p95 ≤ 1000 ms → PASS。
- 首帧规则（客户区 0/30/100/300 ms 纯 RGB 黑 <0.1%）全部 0%；800 ms 稳定帧黑像素 0 → PASS。
- 5 次均 `normal_close=true`、`temp_root_removed=true`；窗口可见前均观测到隐藏窗口（无白闪/黑闪伪影）。

## 2. 真实输入与导出

`tools/walkthrough.ps1 -EvidenceMode input`（SendKeys 真实键入，非注入 API）：

- 键入并保存：文档 **40 B**，SHA `CDE05421…AD715C`；
- 撤销恢复原始内容：`undo_restored_original_bytes=true`（原始 SHA `54AB3751…50E2212`）；
- HTML 导出 **2,085 B**，SHA `CB44D90C…B6EE27E`，内容断言 `export_html_content=true`；
- 状态 **PASS**，正常关闭、临时根已清理。

## 3. 20k 击键（两轮）

`cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_20k_blocks`，各采样 300 次：

| 轮次 | n | p50 ms | p95 ms | max ms | exit | test result |
|---|---|---|---|---|---|---|
| run1 | 300 | 2.537 | 2.907 | 3.864 | 0 | ok |
| run2 | 300 | 2.519 | 2.958 | 3.826 | 0 | ok |

- 最大 p95 = **2.958 ms** ≤ 16 ms → PASS；两轮日志见 `perf/run1.log`、`perf/run2.log`。

## 4. 150% 八表面评分（逐张目检）

评分项：内容、顺序、裁切、重叠遮挡、对比度、字号、表格、图片、公式、Mermaid；阈值 **≥4.5**。

| 表面 | 证据 | 最低分 | 判定 |
|---|---|---|---|
| light 编辑态 | `visual/light/app/02-step-edit-00-…` … `18-step-edit-bottom-…` | 5 | PASS |
| light 源码态 | `visual/light/app/19-step-source-00-…` … `28-step-source-bottom-…` | 5 | PASS |
| light HTML | `visual/light/html/02-step-html-00-…` … `14-step-html-bottom-…` | 5 | PASS |
| light PDF | `visual/light/pdf/02-step-pdf-00-…` … `09-step-pdf-bottom-…` | **4.5（字号）** | PASS |
| dark 编辑态 | `visual/dark/app/02-step-edit-00-…` … `18-step-edit-bottom-…` | 5 | PASS |
| dark 源码态 | `visual/dark/app/19-step-source-00-…` … `28-step-source-bottom-…` | 5 | PASS |
| dark HTML | `visual/dark/html/02-step-html-00-…` … `14-step-html-bottom-…` | 5 | PASS |
| dark PDF | `visual/dark/pdf/02-step-pdf-00-…` … `09-step-pdf-bottom-…` | **4.5（字号）** | PASS |

- **全项最低分 4.5**（仅 PDF 甘特图日期字号，150% 原生物理像素下仍可辨），其余各项 5。
- 内容完整：标题层级、双列表、嵌套列表、三列表格（含换行长单元格）、本地 PNG、行内/块公式、4 张 Mermaid
  （流程图/时序/甘特/思维导图）、`FINAL-SENTINEL` 均按顺序出现；编辑态、源码态、HTML、PDF 四路一致。
- 裁切/重叠：编辑态与 HTML 无内容裁切、无叠字；PDF 中 `04`、`07` 标题落在页末、图体在次页（孤立标题，内容未丢），
  PDF 与 `evidence/render/task9-final/` 逐字节一致，属既有分页现象，不扣裁切/顺序分。
- 对比度：dark 甘特条为浅灰填充 + 深灰标签（放大确认可读）、日期为灰字深底；其余正文/表格/代码对比清晰。
- Mermaid：四图均本地渲染、无外链、无缺失；思维导图双主题配色正常。

## 5. HTML / PDF 与 task9-final 对比

| 主题 | 类型 | 本轮字节 / SHA-256 | 基线 SHA-256 | 结论 |
|---|---|---|---|---|
| light | HTML | 74,703 B / `A69DD813…790EDCBAB` | `F8361A07…BBE8CC01` | 仅今天线变化（见下） |
| light | PDF | 27,774,154 B / `8120795C…67725D1BB` | `8120795C…67725D1BB` | **逐字节相同** |
| dark | HTML | 78,577 B / `0B97D1BA…930DFF2B` | `AE94D083…9CA070B1` | 仅今天线变化 |
| dark | PDF | 27,774,154 B / `8120795C…67725D1BB` | `8120795C…67725D1BB` | **逐字节相同**（与 light PDF 亦相同） |

HTML 差异（两主题均 **4 字节**，其余全部相同）：

- light 偏移 `53288,53289,53299,53300`；dark 偏移 `56860,56861,56871,56872`；
- 位置：`<g class="today"><line class="today" ... x1="21160" x2="21160">` → `x1/x2 = 21188`（+28 px）；
- 原因：甘特图「今天」竖线随导出时刻移动（17:49→19:34），属预期；无任何排版/内容差异。

## 6. 方法与限制

- 截图：独立 Edge profile + 独立 `VEIL_HOME`，`PrintWindow(PW_RENDERFULLCONTENT)` 原始物理像素；每表面完整滚动覆盖内文。
- 应用两主题导出 exit 0；查看器截图 exit 0；本轮结束后无残留 veil/msedge 进程。
- 未改任何 tracked 文件，未 commit / push；只写 `evidence/ux/final-v4/`（及任务 A 的 `after-v6/`）。

**needs-user（未运行、未预填 PASS）**

1. 真实 100% DPI 的 light/dark × 编辑态/源码态/HTML/PDF 八表面：**NOT RUN**，未打分、未由 150% 推断。
2. 物理打印（纸张、份数、页码范围、取消）：**NOT RUN**。

**FAIL**：无。
