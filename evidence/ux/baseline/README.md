# UX 现状基线（spec: polish-veil-ux-craft / Task 0）

本目录只含证据，不改 tracked 文件、不 commit。所有截图与日志为原始文件。

## 环境与口径

- 被测产物：`D:\VSCode\Veil\target\release\veil.exe`
  - 字节数：13,998,592 B
  - SHA-256：`58C9AD0DECFD614DA73098A006AC0773B2515CD49C56684B3F1AEC5FD0D4AB75`
  - 与 `evidence/startup/task9-final/{default,d1}/summary.json` 记录的 `58c9ad0d…ab75` 一致。
- 系统 DPI：144（150%），`HKCU\Control Panel\Desktop\WindowMetrics\AppliedDPI = 144`。
- 物理屏幕：`2560 x 1440`，物理工作区 `2560 x 1368`（任务栏占底部 72 px，DPI 感知实测）；
  150% 下的逻辑读数约 `1707 x 960` / `1707 x 912`。原记录的「物理屏 1707x960 / 工作区 1707x912」
  为逻辑值误记，更正见 `facts.md` 的 Corrigendum。
- 采集工具：`tools/walkthrough.ps1`（本次全部场景均用其 `-Widths/-Steps/-ShotDir` 能力，无自造步骤）。
- 隔离：每个场景独立 `VEIL_HOME`、`APPDATA`、`LOCALAPPDATA`、`TEMP/TMP` 到本目录 `_home/`、`_work/temp/`；
  临时文档全部位于 `_work/`，未打开真实用户文档；未启动第二个实例。
- 窗口几何：为了在 2560x1368 的物理工作区内拍到完整窗口，`_home/*/config.toml` 设
  `window_width=900, window_height=560, window_x=40, window_y=20`（逻辑像素）。
  宽度保持产品默认 900 逻辑（150% 下客户区 1350 物理）；高度被限到 560 逻辑，系当时按
  DPI 未感知读数 1707x912 判断默认 700 逻辑过高（真实物理工作区 2560x1368，此限制可放宽）。
  默认高度下的实拍异常见 `a-empty-default-900x700-pilot/`。
- 截图方式：`PrintWindow(PW_RENDERFULLCONTENT)`；当 walkthrough 的“空白帧”启发式误判时，
  日志会写明并降级为置顶屏幕捕获（原文件里 `walkthrough.log` 有 WARN 行）。

## 场景表

| ID | 场景 | 窗口客户区（物理/逻辑） | 截图 | 状态栏左 | 状态栏右 | 通知 | 模态 | 结论 |
|----|------|------------------------|------|----------|----------|------|------|------|
| a | 空文档首跑（默认宽度；隔离首启，Ctrl+N） | 1350 / 900 | `screenshots/a-empty-first-run/01-w1350-p0-20261121-103151.png`（首启 SAMPLE）、`.../02-step-empty-first-run-20261121-103151.png`（空白草稿） | 01：`未命名草稿`；02：`未命名草稿`，均完整 | 01：`181 字 · 行 1，列 1`；02：`0 字 · 行 1，列 1`，完整 | 02 中段可见`已新建空白文档` | 无 | 通过；空文档可正常进入，状态栏左右不重叠 |
| b | 窄窗口约 900 px 的空文档 | 900 / 600 | `screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-103227.png` | `未命名草稿`，完整 | `0 字 · 行 1，列 1`，完整 | `已新建空白文档` 可见 | 无 | 通过；900 物理宽度下菜单/状态栏无重叠 |
| c | 深目录 + 长文件名 + 脏态 | 1350 / 900 | `screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-20261121-103258.png` | `2026-11-21-ux-baseline-…-abcdefghijklmnop.md · 第四层 * · 未保存`，完整（未截断） | `29 字 · 行 5，列 23`，完整 | 无 | 无 | 通过；状态栏只显示“文件名 · 直接父目录”，不显示完整深路径 |
| d | 保存失败通知（目标不可写，Ctrl+S） | 1350 / 900 | `screenshots/d-save-fail/02-step-save-fail-modal-20261121-103337.png`（模态）、`.../03-step-save-fail-notice-20261121-103337.png`（通知） | `save-fail.md · failsave * · 未保存`，完整 | `19 字 · 行 5，列 12`，完整 | 中段可见`保存失败：拒绝访问。 (os error 5) （写入 D:\V…）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`（中段省略路径） | `保存失败` + 原因/下一步 + `重试 (R)`/`另存为`/`取消` | 通过；模态与状态栏通知同时出现，通知在遮罩下仍可读（偏暗） |
| e | 未保存关闭对话框（脏态关窗） | 1350 / 900 | `screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-103410.png` | `unsaved-close.md · unsaved * · 未保存`，完整 | `20 字 · 行 5，列 12`，完整 | 无（中段空白） | `是否保存对「unsaved-close.md」的更改？` + `未保存的修改在关闭后将无法恢复。` + `保存 (S)`/`不保存 (C)`/`取消` | 通过；WM_CLOSE 被拦成应用内模态，窗口未关闭 |

## 自动采集能力结论

- 5 个要求场景全部由 `tools/walkthrough.ps1` 自动采集成功，没有“不可自动采集”的场景。
- 限制与旁证：
  - 空白文档的 PrintWindow 结果会被 walkthrough 的空白帧检测判为 blank，自动降级为屏幕捕获；
    这是采集脚本的启发式，不是产品未绘制。日志 WARN 已保留。
  - 场景 d 的通知位于模态遮罩之下，属“可见但被压暗”；正文与按钮以模态截图为准。
  - 场景 e 用 `close`（WM_CLOSE）而非 Alt+F4，两者都走 `WindowEvent::CloseRequested`；
    Alt+F4 未单独实拍。
  - 场景 c 在 1350 物理宽下长文件名完整显示，未触发中段省略；窄宽度下的省略未在本轮组合采集。
    人工补测步骤：`pwsh -NoProfile -File tools/walkthrough.ps1 -Doc "<长文件名.md>" -Widths 900 -Steps "wait:800;click:300,300;wait:200;type:dirty;wait:600;shot:long-name-900" -StepsWidth 900`。

## 150% 视觉与性能对照（引用既有证据，不重跑长测）

| 来源文件 | 文件 SHA-256 | 绑定产物 | 关键数字 |
|---|---|---|---|
| `evidence/render/task8a-b/summary.json` | `0B3BA6F365751F39ADC4B953FF54245BE6EFF999DE458F5045A49F960DF967A0` | 14,005,760 B / `BD01E816…1A811DBC`（**与当前 exe 不同**） | status=PASS；150% 真实 DPI（144）；PrintWindow 原生像素；8 个表面（light/dark × editor/source/html/pdf）全部 ≥4.5，均值 4.95–5.0；90 张评分截图；app 窗口 1422x1106 物理 |
| `evidence/startup/task9-final/default/summary.json` | `4880C35DDAE6D09A655F2308DB5AEDB2A152E269F55FB875D51B8031A88F11F1` | 13,998,592 B / `58c9ad0d…ab75`（与当前一致） | 5 次启动到窗口：p50 587.25 ms、p95 640.78 ms、max 640.78 ms、min 555.6 ms；首帧黑像素 0/1,417,500（5 次全 0）；DPI=144 |
| `evidence/startup/task9-final/d1/summary.json` | `9F886DAB59DDE3140F84022760CDD523DA695C7A4BF5AE2B51DE402FE123D506` | 13,998,592 B / `58c9ad0d…ab75`（与当前一致） | 30 个样本：p50 102.7 ms、p95 119.65 ms、max 144.72 ms、mean 103.51 ms；idle WorkingSet 43.71/44.03/44.02 MiB；3 次重复全部 PASS |

注意：`task8a-b` 的视觉结论属于旧构建（SHA 不同），不能直接当作当前二进制的视觉回归结论；
本轮未重建 exe，也未重跑长测。

## 文件清单

- `README.md`：本文件（环境、场景表、引用与限制）。
- `facts.md`：逐场景事实 + 代码行号证据。
- `screenshots/<场景>/`：原始 PNG + `walkthrough.log` / `stdout.log` / `stderr.log`。
- `_work/`：临时文档与不可写目标（场景 d 的 `save-fail.md.veil-tmp/` 目录）。
- `_home/`：隔离的 VEIL_HOME（config/session/snapshots/recovery）。
