> **HISTORICAL（历史存档，已被取代）**
>
> - 本报告绑定 commit `7c00765` 与 exe SHA-256 `25fea5f3…2b67`，仅记录该提交的历史复验结果。
> - 已被 `evidence/ux/final-v2/report.md` 取代，**不得**再作为「最终」结论引用，也不得再自称最终。

# spec `polish-veil-ux-craft` 量化复验（历史）

- 结论：**PASS**（启动 p95 544.76 ms ≤ 1000 ms；首帧黑像素 0%；20k 击键 p95 2.769 ms ≤ 16 ms；8 个表面全部单项 ≥ 4.5）
- 复验日期：2026-11-21，时区 Asia/Shanghai
- 证据目录：`evidence/ux/final/`（gitignore），本报告外的所有原始截图、日志、导出件均保留在该目录

## 1. 被测产物校验

| 项目 | 值 |
|---|---|
| 路径 | `d:\VSCode\Veil\target\release\veil.exe` |
| 字节数（开始/结束） | 14,010,368 / 14,010,368 |
| SHA-256（开始/结束） | `25FEA5F3A65430211F05D9BFA2EC0667672C88D31AAC66F90DE4BC56ACEE2B67`（一致） |
| commit | `7c00765` |
| 机器可读记录 | `evidence/ux/final/exe-verify-start.txt`、`exe-verify-end.txt` |

环境：Windows，物理屏幕 2560×1440，逻辑 1707×960，系统 DPI **144（150%）**；六个表面采集日志均记录 `真实窗口 DPI=144（比例=1.5）`。截图方式为 `tools/walkthrough.ps1` 的 `PrintWindow(PW_RENDERFULLCONTENT)` 原始物理像素，未缩放。

未写入任何 tracked 文件；worktree 中 `CHANGELOG.md`、`README.md`、`README.zh-CN.md` 的改动在本任务开始前已存在，本任务未触碰。

## 2. 任务 1：5 次冷启动（startup 模式 + 隔离 VEIL_HOME）

命令（每次独立临时 `VEIL_HOME`/`APPDATA`/`TEMP`/文档，串行执行）：

```
pwsh -NoProfile -File tools/walkthrough.ps1 -EvidenceMode startup -ShotDir evidence/ux/final/startup/run-0N
```

| run | start_to_window_ms | 首帧黑像素（0/30/100/300/800 ms） | 正常关窗 | 临时根清理 | 退出码 |
|---|---:|---|---|---|---:|
| 01 | 514.21 | 0 / 0 / 0 / 0 / 0（占 1,417,500 像素的 0%） | True | True | 0 |
| 02 | 531.78 | 0 / 0 / 0 / 0 / 0 | True | True | 0 |
| 03 | 547.01 | 0 / 0 / 0 / 0 / 0 | True | True | 0 |
| 04 | 535.78 | 0 / 0 / 0 / 0 / 0 | True | True | 0 |
| 05 | 519.85 | 0 / 0 / 0 / 0 / 0 | True | True | 0 |

- p50 = **531.78 ms**，p95（PERCENTILE.INC，n=5）= **544.76 ms**，max = **547.01 ms**，mean = 529.73 ms。
- 断言 p95 ≤ 1000 ms：**PASS**；首帧黑像素 < 0.1%：**PASS（五轮、五个采样点全部为 0%）**。
- `start_to_window_ms` 采用 startup 模式的 `launch_to_visible_ms`：2 ms 轮询到窗口已显示、可见且位于屏幕内的时间点（另含隐藏窗口观测记录）。
- 证据：`evidence/ux/final/startup/run-01…05/dpi-150/{summary.json,frames.csv,*.png}`。

## 3. 任务 2：真实窗口输入

```
pwsh -NoProfile -File tools/walkthrough.ps1 -EvidenceMode input -ShotDir evidence/ux/final/input
```

退出码 0，`status=PASS`，使用 `System.Windows.Forms.SendKeys.SendWait` 向真实窗口输入：

| 断言 | 实测 |
|---|---|
| 键入并保存 | 38 B，SHA-256 `5464B4F1…45B0` PASS |
| 撤销恢复 | 30 B，SHA-256 `54AB3751…2212` PASS |
| 导出前文档 | 40 B，SHA-256 `CDE05421…5C` PASS |
| `Ctrl+E` HTML 导出 | 2,085 B，断言 `doctype+base+VEILEXPORT` PASS，SHA-256 `CB44D90C…27E` |
| 正常关窗 | PID 14824 ExitCode=0，forced=False |

文档为当次临时目录中的独立副本（标题栏 `corpus.md`），临时根 `veil-task6-…` 已删除；未打开或改写真实用户文档。证据：`evidence/ux/final/input/input/{summary.json,assertions.csv,export-final.html,*.png}`。注：该模式的 `exported.png` 曾因 PrintWindow 空白降级为屏幕捕获（不影响本节断言，也不用于视觉评分）。

## 4. 任务 3：20k 击键（release）

```
cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_20k_blocks
```

| run | samples | p50 | p95 | max | 退出码 | 日志 |
|---|---:|---:|---:|---:|---:|---|
| 1 | 300 | 2.441 ms | **2.769 ms** | 2.970 ms | 0 | `evidence/ux/final/perf/run1.log` |
| 2 | 300 | 2.367 ms | **2.687 ms** | 3.796 ms | 0 | `evidence/ux/final/perf/run2.log` |

断言 p95 ≤ 16 ms：**PASS**（两轮均 ≤ 2.77 ms）。

## 5. 任务 4：150% 视觉 rubric（8 个表面）

采集：隔离 `VEIL_HOME`（`config.toml` 的 `mode`）与 `APPDATA`/`TEMP`，把 `evidence/render/shared/corpus.md` + `veil-logo.png` 复制到临时工作目录；应用内滚动覆盖全部 Mermaid 后经 GUI `Ctrl+E` / `Ctrl+Shift+E` 与原生对话框导出；HTML/PDF 在独立 Edge profile 的 150% DPI 窗口中打开逐屏截图。应用 1422×1106，HTML 1422×1338，PDF 1222×1338 物理像素，日志均记录 DPI=144。应用两次退出码 0、关闭为 `CloseMainWindow` 正常退出；查看器截图成功（首次因参数传递使用相对路径失败，改用绝对路径后重采，未使用失败批次图片）。

评分口径与 `evidence/render/task8a-b/rubric.md` 一致：10 个单项，允许 0.5 分，**任一单项 < 4.5 即 FAIL，不用均值抵消**。逐张目检，末尾之后的重复视图已核对为同一内容。

| 主题/表面 | 内容 | 顺序 | 裁切 | 重叠/遮挡 | 对比度 | 字号 | 表格 | 图片 | 公式 | Mermaid | 均值 | 最低 | 门槛 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| light/编辑态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| light/源码态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| light/HTML | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| light/PDF | 5 | 5 | 5 | 5 | 5 | **4.5** | 5 | 5 | 5 | 5 | 4.95 | **4.5** | PASS |
| dark/编辑态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| dark/源码态 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| dark/HTML | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5 | 5.00 | 5 | PASS |
| dark/PDF | 5 | 5 | 5 | 5 | 5 | **4.5** | 5 | 5 | 5 | 5 | 4.95 | **4.5** | PASS |

**8 表面最低单项 = 4.5（light/dark PDF 字号），全部 ≥ 4.5，无回归，总判定 PASS。**

证据（均含 `walkthrough.log` 与逐屏 PNG）：

- light 编辑态：`evidence/ux/final/visual/light/app/02-step-edit-00-20261121-132402.png` … `17-step-edit-bottom-…png`
- light 源码态：`…/light/app/18-step-source-00-…png` … `26-step-source-bottom-…png`
- light HTML：`…/light/html/02-step-html-00-20261121-133314.png` … `13-step-html-bottom-…png`
- light PDF：`…/light/pdf/02-step-pdf-00-20261121-133345.png` … `09-step-pdf-bottom-…png`
- dark 编辑态 / 源码态：`…/dark/app/02-step-edit-00-20261121-132631.png` …、`18-step-source-00-…png` …
- dark HTML / PDF：`…/dark/html/02-step-html-00-20261121-133414.png` …、`…/dark/pdf/02-step-pdf-00-20261121-133445.png` …

关键复核（原图裁切在 `evidence/ux/final/visual/_crops/`）：

- 表格：三列完整，Gamma 的 `code` 未断成 `cod`/`e`，边框与左/中/右对齐一致（`light/app/03-step-edit-01…`、`dark/app/03-…`）。
- 本地图片、行内公式 `E=mc²` 基线与块公式根号/分数线完整（`light/app/04-step-edit-02…`、`dark/pdf/04-…`）。
- 4 个 Mermaid 渲染完整；思维导图根节点 `Veil` 文字完整居中；流程图“是/否”边标签与箭头无文字遮挡；甘特图条形/分色/标签完整（`dark/html/04-step-html-02…`、`dark/pdf/06-step-pdf-04…`）。
- 甘特日期在 PDF 中偏小但原始物理像素下可辨，故字号记 4.5（不是低分项）；暗色 HTML/编辑态的日期在原生像素裁切图 `_crops/dark-html-gantt-dates.png`、`dark-app-gantt3.png` 中清晰可读。
- 源码态代码块首尾围栏完整，甘特块底部裁剪已在原生像素核对（`_crops/gantt-fence-new-light-22.png`），未复现上一轮疑似截断。
- 末尾 `08 顺序终点 / FINAL-SENTINEL` 在 8 个表面均完整可见。

### 5.1 导出件与 `evidence/render/task9-final/` 对比

| 主题/类型 | 本轮字节 | 本轮 SHA-256 | task9-final SHA-256 | 结论 |
|---|---:|---|---|---|
| light HTML | 74,703 | `24DD67F34A69A339597C1AB48BA7968BF7F3F4E684E6214616A7E7C4A4AF93D8` | `276467BD36C9A16CD142B30DEEA81F0A3602E42F109F8BB6548AAE5A013E9147` | 仅 today 线差 4 字节 |
| light PDF | 27,774,154 | `8120795C45632B75373AEBCCBD22BE740E34C1E5EA9C9140A946A0467725D1BB` | 同左 | **字节级一致** |
| dark HTML | 78,577 | `B667DBE6CAF5E51DDFF5BFCD36005D93C632BB6F6B8C6697CAFE56B919855B98` | `5147145E18512B2DD43894EAE4F5D16EFEB86384DD82517C52DA744F8772421A` | 仅 today 线差 4 字节 |
| dark PDF | 27,774,154 | `8120795C45632B75373AEBCCBD22BE740E34C1E5EA9C9140A946A0467725D1BB` | 同左 | **字节级一致** |

HTML 差异实测（逐字节比对）：

- light：共 4 个差异字节，偏移 `53288..53289`、`53299..53300`；dark：4 个，偏移 `56860..56861`、`56871..56872`。
- 差异位置唯一：`06 Mermaid 甘特图` 内 `<g class="today"><line class="today" … />` 的 `x1/x2`，由基线 **21160** 变为 **21171**（右移 11 px），其余全部字节相同；符合“today 线随导出时刻移动”的允许项。截图层面除该竖线位置外无可视差异。
- PDF 差异：0 字节，light/dark 均与 task9-final 完全一致（含目录页与 4 页分页）。

### 5.2 非阻断观察（未计为低分项）

- PDF 第 2 页末尾仅有标题 `04 Mermaid 流程图`、第 3 页末尾仅有标题 `07 Mermaid 思维导图`，图体在下一页，页底留白较大。因 PDF 与 task9-final 字节级一致，属既有分页现象；标题与图顺序正确、无裁切，故不改动评分。
- dark 主题 PDF 使用浅色打印图（与 task9-final 相同）。
- 编辑态活动块显示源码 reveal 属既定行为，按 rubric 不扣分。

## 6. needs-user（NOT RUN，未打分）

- 真实 100% DPI 的 light/dark × 编辑态/源码态/HTML/PDF：**needs-user / NOT RUN**，未由 150% 结果推断。
- 物理打印（纸张、份数、页码范围、取消）：**needs-user / NOT RUN**，未预填 PASS。

## 7. 产物

- `evidence/ux/final/summary.json`：含产物校验、5 次启动逐帧黑像素、两次 perf、8 表面逐项分数、导出件哈希、HTML 差异偏移、命令、needs-user，以及 238 个证据文件（截图/日志/脚本）的字节数与 SHA-256。
- `evidence/ux/final/report.md`：本文件。
- 采集脚本（可复现）：`evidence/ux/final/_scripts/{visual.ps1,edge-capture.ps1,write-summary.ps1}`。
- 结束复核：exe 字节数/SHA-256 与开始时一致；无残留 Veil/Edge 进程。
