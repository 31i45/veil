# after-v5 事实记录（最终产物 ce038eb）

产物 `target/release/veil.exe`：14,010,880 B，SHA-256
`FC6BCCD4A11C0214E07974F6227F02BF2C10063E98CE6917B1E6303F14F9D01C`，提交 `ce038eb`。
开始 17:30:12 / 结束 17:41:17 两次校验一致（`exe-verify.txt`，350 B，SHA
`711A5B3D551242133261D7D65C27EC38A184BD32CA3FDAB75D6E6EEA6CFC0A1A`）。
DPI 144（150%）；物理屏 2560x1440；工作区 `0,0,2560,1368`；默认窗口外框 1372x1106 @593,127（客户区 1350x1050 = 900x700 逻辑）。

v4（77b88e2）→ v5（ce038eb）唯一产品改动：`app.rs` 增加 `MIN_WINDOW_SIZE=(520,360)`、
`with_min_inner_size` 与恢复路径 `max()` 钳制；`app_tests.rs` 增加 2 个测试。i18n 无改动，因此全部文案与 v4 相同。

## F1 十个场景（v4 → v5）

| # | 场景 | v4 | v5 | 证据（相对 screenshots/） |
|---|---|---|---|---|
| 1 | 空文档首跑 | `未命名草稿 ｜ 已新建空白文档 ｜ 0 字 · 行 1，列 1` | 同文（逐字节同 v4 布局行为） | `a-empty-first-run/03-step-empty-first-run-20261121-173017.png`（26,159 B，SHA `EB4CF3B7…2CA96386`） |
| 2 | 900px 窄窗 | 左右与通知完整 | 同文，无重叠 | `b-narrow-900/02-step-empty-narrow-900-20261121-173025.png`（24,342 B，SHA `041623A2…CF169E2`） |
| 3 | 700px 长名脏态 + 错误通知 | 5 行浮层完整 | 浮层 6 行完整（不省略、不裁切）；`…stuvwxyz.md · 第四层 · 未保存 ｜ 28 字 · 行 4，列 25` | `f-700-longname-notice/02-step-700-longname-notice-20261121-173037.png`（69,667 B，SHA `D0311756…85909DCF`） |
| 4 | 保存失败通知 | 模态 + 2 行通知 | 模态（原因/下一步/重试(R)/另存为/取消）+ 2 行常驻通知 | `d-save-fail/02-step-save-fail-modal-20261121-173057.png`（82,069 B，SHA `5C894110…936E5F19`）、`03-…png`（48,138 B，SHA `2F56AF35…C6ADF4CE`） |
| 5 | 未保存关闭 | 三行逐按钮说明 | 同文：`[保存]…/[不保存]…/[取消]…` + `保存(S)/不保存(C)/取消` | `e-unsaved-close/02-step-unsaved-close-modal-20261121-173112.png`（61,061 B，SHA `104D4BBF…F84F33`） |
| 6 | 文件菜单 | 动作名/快捷键/禁用原因 | 同文：`导出 HTML…/导出 PDF…/打印…` 均带「空文档没有可导出的内容」 | `g-file-menu/02-step-file-menu-open-20261121-173119.png`（54,461 B，SHA `EC05CC08…C8ECA165`，与 v4 同 SHA） |
| 7 | 浅色焦点环 | 可见 | 可见（蓝环） | `h-search-light/02-step-search-light-20261121-173145.png`（99,824 B，SHA `1AA0D99C…80096A75`） |
| 8 | 深色焦点环 | 可见 | 可见（输入框底色 #333A44） | `i-search-dark/02-step-search-dark-20261121-173152.png`（93,416 B，SHA `E8B5636A…AABB0E9F`） |
| 9 | 整窗 / 工作区 | 1372x1106 @593,127；工作区 2560x1368 | 同（已复测） | `j-full-window/02-…png`（95,617 B，SHA `C3A86FCB…DBACD1B`）、`j-full-window/window-rect.txt` |
| 10 | 最小尺寸 | v4 无最小尺寸（140×160 可生效） | 见 F2：用户拖拽/恢复钳到 520×360；程序化 SetWindowPos 仍可降到 140×160 | `m-min-size/*`、`window-min-size.txt` |

## F2 最小尺寸 520×360（详见 window-min-size.txt）

- `[A]` SetWindowPos 目标客户区 140×160 物理 → 实际客户区 140×160（93.33×106.67 逻辑），**未钳制**。
- `[B]` SetWindowPos 目标 140×160 逻辑（210×240 物理）→ 实际 210×240（140×160 逻辑），**未钳制**。
- `[C]` WM_GETMINMAXINFO `ptMinTrackSize=802×596` 外框 → 客户区下限 780×540 = 520×360 逻辑。
- `[D]` 真实鼠标从右下角边框拖拽 → 客户区精确停在 780×540（520×360），`clamped=True`。
- `[E]` config 记录 480×360 启动 → 客户区 780×540；config 93×107 低于解析下限被整组忽略 → 默认 900×700。
- `[F]` 最小尺寸（520×360）下保存失败通知：首行「保存失败：拒绝访问。 (os error 5)（写入」与末行
  「…；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为」均完整可见；截图 63,739 B，SHA `5559AF64…8625A7B8`。
- 单元测试（`screenshots/tests.txt`）：
  `app_tests::min_window_size_keeps_four_notice_lines_and_status_readable` ok；
  `app_tests::initial_window_never_falls_below_min_size_on_tiny_work_area` ok；
  `chrome::tests::notice_overlay_layout_keeps_rows_in_client_area_and_first_last` ok。
- 判定：**用户路径 PASS；任务要求的“SetWindowPos 设为 140×160 被钳制” FAIL**（Windows 仅对用户 sizing
  应用 tracking size，应用未自钳制）。

## F3 错误通知常驻（t=0 与 t=7s 逐字节一致）

- 链接不支持：文档 `[unsupported-link-target-abcdefgh](ftp://example.com)`，Ctrl+点击（`_work/link.ps1`）。
  状态栏 `无法打开链接：不支持的链接地址 ftp://example.com`。
  `l-link-unsupported/notice-t0.png` 与 `notice-t7s.png` 均 38,680 B，SHA `E44EDF8B…FDFB704`，`byte_identical=True`。
- 图片格式不支持：Ctrl+Shift+I → 原生选图对话框 → 粘贴非图片路径 `notimage.txt` → 回车（`_work/image-notice.ps1`）。
  状态栏 `所选文件不是支持的图片格式；下一步：改用 PNG/JPEG/WebP 等支持的格式`。
  `l2-image-unsupported/notice-t0.png` 与 `notice-t7s.png` 均 332,999 B，SHA `9E0F379E…AF308F55`，`byte_identical=True`。
- 方法说明：图片场景首次取图（未滚动）两图仅差 104 px（bbox 35,114–36,165，宽 2 px 的编辑光标闪烁），
  通知本身不变；最终版把插入点滚出视口（应用对不在视口内的光标不绘制）后取得逐字节一致的两张图，
  未对像素做任何修改。

## F4 打印 / PDF 失败回执

文案（HEAD，`i18n.rs:407-420`）：

- `打印失败：{}；可重试打印，或检查打印机是否联机、驱动是否正常后重试`
- `导出 PDF 失败：{}；可重试导出，或检查目标目录是否可写、文件是否被占用后重试`
- `打印失败：无法获取打印机上下文；请检查打印机连接后重试，或改用导出 PDF`

单测（`screenshots/tests.txt`，2,757 B，SHA `50F01FDE…D3074AB12E`）：
`print_and_pdf_failure_notices_are_errors_and_survive_ttl`、`no_printer_context_notice_is_error_with_next_step_and_survives_ttl`、
`i18n::tests::print_failures_have_actionable_next_step` 全部 ok（均断言 Error 类 + 含下一步 + 跨 6s TTL）。

运行时回执：**不可自动采集**（未伪造）。尝试记录：Ctrl+Shift+E 打开原生「导出 PDF」保存对话框，
把文件名指向只读已存在文件 `locked.pdf`，对话框弹出覆盖确认（549×214，已检测到）后仍拒绝落盘、
原生对话框保持打开，应用层失败回执未被触发。证据 `screenshots/print-pdf/attempt-metrics.txt`、
`attempt-native-save-dialog.png`。人工复现：在导出对话框选择只读/无权限目标目录，或停用默认打印机后 文件→打印…。

## F5 方法与限制

- 全部截图走 `tools/walkthrough.ps1`（PrintWindow，失败时降级置顶屏幕捕获）；每次运行独立 `VEIL_HOME`
  （配置与数据均落在 `_home/<场景>/`），未触碰真实用户配置。
- 极窄窗/最小尺寸、链接点击、图片通知、PDF 尝试使用 `_work/` 内一次性驱动脚本（由 v4 的
  `extreme.ps1`、`link.ps1` 与 final-v2 的 `visual.ps1` 结构改写，路径全部指向 after-v5）。
- 只读文档（f/d/k/m）在运行期 `attrib +R`、结束后恢复为 Archive；未改仓库 tracked 文件。
