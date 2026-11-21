# after-v4 事实记录（最终产物 77b88e2）

产物 14,010,880 B，SHA-256
`C24599BED259E1B96BB531169FD87D2F4254207CDAB4FB9BB7320FC451CC1269`，提交 `77b88e2`。
开始/结束一致（`exe-verify.txt`，336 B，SHA `B1FB741428AD94984D41772388DA94621453B381E59F90B89F85033872A6D26F`）。
DPI 144；物理屏 2560x1440；工作区 `0,0,2560,1368`；逻辑 1707x960/912。

## F1 140×160 极窄窗（保存失败浮层）

**口径**：项目测试矩阵用 140×160 逻辑 @150%（`chrome/tests.rs:1381`），即客户区 210×240 物理。
walkthrough 只能设宽，故用 `_work/extreme.ps1` 将客户区精确设为 210×240。

截图 `screenshots/n2-140x160-logical/notice.png`（13,430 B，SHA
`902FA78E63520626E5D0346275DE3719AACE2946F637B09EBF7CF5609D9F1D9C`），
放大 `notice-zoom-x2.png`（11,257 B，SHA `E390534683D00A9FF38E1E3E4FA0109430240E09287A899A970575E962B044A7`）。
测量 `metrics.txt`（509 B，SHA `DC37BB14F8B83EFC4492B3A4F9969A574D0D19DEF790CF9F12F758B73296C8D1`）。

- 外框 232×296，客户区 210×240。
- 浮层卡片 `y=54..201`（h=147），`line_h=27`；行槽
  `60..87 / 87..114 / 114..141 / 141..168 / 168..195`，全部 `< 201`（状态栏上沿）且 `< 240`。
- 可见首行 `保存失败：拒绝`、可见末行 `另存为`，无半行裁切；中间行按“首末优先”让位。
- 测试 `notice_overlay_layout_keeps_rows_in_client_area_and_first_last`
  （`chrome/tests.rs:1378`）对 (140,160,1.0) 与 (210,240,1.5) 断言：行矩形不越界、首行与末行必须保留。
- 判定：**PASS（逻辑口径）**。

附加更严苛记录（不参与主判定）：`screenshots/n3-140x160-physical/`
（客户区 140×160 **物理**，即 93×107 逻辑；PNG 7,248 B，SHA
`83DD68747C80AB77951628FCA15B83E9587391DA3D4569665AB3141074BB6F06`；
metrics 503 B，SHA `0B16B967898D656BD6252C5ED1004D02E51562816E97476B79D25BD33921FADE`）。
此处只有 2 个行槽，截图仅首行 `保存失败`，第二槽为空白、未见末行 `另存为`。
若验收把 140×160 解释为物理像素，则本项应判 **FAIL**；按项目 DPI/测试口径（逻辑）为 PASS。
另 `n-140x160-notice/` 是 walkthrough 只能改宽的废弃预跑（高仍 1106），不作结论。

## F2 链接不支持错误常驻

- 文档首行 `[unsupported-link-target-abcdefgh](ftp://example.com)`；Ctrl+点击（`_work/link.ps1`）。
- t=0：`screenshots/l-link-unsupported/notice-t0.png`（38,461 B，SHA
  `26A1BF982C367D3FED0F4C184F1FD1BC5A62707B556CE75EAD946A4104060BF1`）。
- t=7s：`notice-t7s.png`（38,461 B，**同 SHA**）。
- 状态栏通知：`无法打开链接：不支持的链接地址 ftp://example.com`。
- 两图逐字节相同，证明未被 6 秒 TTL 清除。代码：`interaction.rs:2876-2882` 走 `notify_error`；
  错误类常驻 `chrome/mod.rs:630-645`、`app.rs:1699-1716`。判定 **PASS**。

## F3 打印 / PDF 失败回执

文案（HEAD 实测）：

- `NoticePrintFailed`：`打印失败：{}；可重试打印，或检查打印机是否联机、驱动是否正常后重试`
  （`i18n.rs:407-408`）。
- `NoticeExportPdfFailed`：`导出 PDF 失败：{}；可重试导出，或检查目标目录是否可写、文件是否被占用后重试`
  （`i18n.rs:412-413`）。
- `PrintNoPrinterContext`：`打印失败：无法获取打印机上下文；请检查打印机连接后重试，或改用导出 PDF`
  （`i18n.rs:419-420`）。

单测（`screenshots/tests.txt`，2,994 B，SHA
`89586E908A784CA91B5DE6743B86B87147C281D732BE8B23F82F4C3387E9F9A0`）：

- `app_tests::print_and_pdf_failure_notices_are_errors_and_survive_ttl`（17496）ok
- `app_tests::no_printer_context_notice_is_error_with_next_step_and_survives_ttl`（17547）ok
- `i18n::tests::print_failures_have_actionable_next_step`（883）ok

均断言 Error 类、含下一步、跨 6s TTL 仍存在。**运行时回执未实拍**：Windows 打印/另存对话框
无法在不改系统打印机、不替用户选路径的前提下稳定注入写失败；不伪造。
人工步骤：停用/离线默认打印机后 文件→打印…；或在导出 PDF 对话框选只读/无权限目录后保存。
判定：**文案与常驻 PASS；运行时截图“不可自动采集”**。

## F4 回归（9 项）

| 场景 | 证据（相对 screenshots/） | 结果 |
|---|---|---|
| 空文档首跑 | `a-empty-first-run/03-step-empty-first-run-20261121-150215.png`（31,686 B，SHA `1599D485…F9B5F86`） | `未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1` |
| 900px 窄窗 | `b-narrow-900/02-step-empty-narrow-900-20261121-150224.png`（26,579 B，SHA `AE6E3983…E1202A`） | 左右与通知完整，无重叠 |
| 700px 长名+错误通知 | `f-700-longname-notice/02-step-700-longname-notice-20261121-150238.png`（70,374 B，SHA `4EC24D5B4AB84A39E40D7B974F27EB3BE922D88C4981440FAA00EC6FDA8B68BE`） | `…s.md · 第四层 · 未保存` ｜ `24 字`；浮层完整无省略号 |
| 保存失败 | `d-save-fail/03-step-save-fail-notice-20261121-150340.png`（49,075 B，SHA `42618C3B…4EC417`） | 2 行浮层完整、常驻；模态见 `02-*`（82,782 B，`D4D5A918…92F2D`） |
| 未保存关闭 | `e-unsaved-close/02-step-unsaved-close-modal-20261121-150401.png`（62,713 B，SHA `746B6711…BC5309`，与 v2/v3 相同） | 三行逐按钮说明 + `保存(S)/不保存(C)/取消` |
| 文件菜单 | `g-file-menu/02-step-file-menu-open-20261121-150333.png`（54,461 B，SHA `EC05CC08…CA165`，与 v2/v3 相同） | 动作名/快捷键/禁用原因完整 |
| 浅色焦点环 | `h-search-light/02-step-search-light-20261121-150409.png`（100,020 B，SHA `7B9EF76F…5961A9`） | 环可见 |
| 深色焦点环 | `i-search-dark/02-step-search-dark-20261121-150415.png`（93,615 B，SHA `CCF47D58…0D90BAE`） | 环可见；输入框底色 #333A44 |
| 整窗 | `j-full-window/02-step-full-window-20261121-150432.png`（95,617 B，SHA `C3A86FCB…BACD1B`）；`window-rect.txt`（110 B，SHA `EF6A749F…471BCE3`） | 窗口 `593,127,1965,1233`；工作区 `0,0,2560,1368`；无任务栏遮挡 |
| 崩溃恢复 | `k-recovery/run2-recovered/02-step-recovery-notice-20261121-150457.png`（65,127 B，SHA `1D9A3DCC…79F526A6`） | 对象 `recover-me.md`、时间 `15:04`、去向「recovery」完整；状态 `… · 未保存` ｜ `23 字 · 行 1，列 1` |

## 结论

- 产品行为无 FAIL：极窄窗（逻辑口径）首末行完整；链接错误常驻；7 项回归与 after-v2/v3 一致。
- 唯一未实拍：打印/PDF 失败回执（原生对话框限制），但文案与常驻由 3 个通过的单测确认。
- 额外风险记录：若把 140×160 当物理像素，末行不可见（`n3-*`），按项目测试口径不应如此解释。
