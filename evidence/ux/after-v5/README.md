# after-v5：最终产物 UX 重采（commit ce038eb）

只写本目录；未改任何 tracked 文件，未 commit / push；`after/`、`after-v2/`、`after-v3/`、`after-v4/`、
`final-v2/` 均未改动。采集沿用 `tools/walkthrough.ps1`，极窄窗/最小尺寸、链接与图片通知由
`_work/` 内脚本（由 after-v4 的 `extreme.ps1`、`link.ps1` 和 final-v2 的 `visual.ps1` 改写）补充。

## 产物校验

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始 17:30 | 14,010,880 | `FC6BCCD4A11C0214E07974F6227F02BF2C10063E98CE6917B1E6303F14F9D01C` | `ce038eb` |
| 结束 17:41 | 14,010,880 | 同上 | `ce038eb` |

`exe-verify.txt`（350 B，SHA `711A5B3D…6FC0A1A`）。DPI 144（150%），物理屏 2560×1440，工作区 `0,0,2560,1368`。

## 判定：FAIL（1 项未满足）

| # | 项 | v4 → v5 | 判定 | 证据 |
|---|---|---|---|---|
| 1 | 空文档首跑 | 文案不变：`未命名草稿 ｜ 已新建空白文档 ｜ 0 字 · 行 1，列 1` | PASS | `screenshots/a-empty-first-run/03-*` |
| 2 | 900px | 不变：左右与通知完整、无重叠 | PASS | `screenshots/b-narrow-900/02-*` |
| 3 | 700px 长名脏态 + 错误通知 | 不变：浮层完整不省略；`…stuvwxyz.md · 第四层 · 未保存 ｜ 28 字`（v5 折 6 行，整段可见） | PASS | `screenshots/f-700-longname-notice/02-*` |
| 4 | 保存失败通知 | 不变：模态含原因/下一步 + 2 行常驻通知 | PASS | `screenshots/d-save-fail/02-*`、`03-*` |
| 5 | 未保存关闭对话框 | 不变：三行逐按钮说明 + `保存(S)/不保存(C)/取消` | PASS | `screenshots/e-unsaved-close/02-*` |
| 6 | 文件菜单 | 不变：动作名/快捷键/禁用原因「空文档没有可导出的内容」完整 | PASS | `screenshots/g-file-menu/02-*` |
| 7 | 浅 / 深焦点环 | 不变：两主题焦点环均清晰 | PASS | `screenshots/h-search-light/02-*`、`i-search-dark/02-*` |
| 8 | 崩溃恢复提示 | 不变：对象/恢复时间/去向三字段完整 | PASS | `screenshots/k-recovery/run2-recovered/02-*` |
| 9 | 整窗 / 工作区 | 不变：窗口 1372×1106 @593,127；工作区 2560×1368 | PASS | `screenshots/j-full-window/02-*`、`window-rect.txt` |
| 10 | 最小尺寸 520×360 | 新增：用户拖拽与恢复路径钳到 520×360；**程序化 SetWindowPos 仍可设到 140×160 / 210×240** | **FAIL**（见下） | `screenshots/m-min-size/*`、`window-min-size.txt` |
| 11 | 错误通知常驻（链接） | 不变：t=0 与 t=7s 逐字节相同 | PASS | `screenshots/l-link-unsupported/notice-t{0,7s}.png` |
| 12 | 错误通知常驻（图片格式） | t=0 与 t=7s 逐字节相同 | PASS | `screenshots/l2-image-unsupported/notice-t{0,7s}.png` |
| 13 | 打印/PDF 失败回执 | 文案与 3 个单测 PASS；运行时回执**不可自动采集**（未伪造） | PASS（文案）/ 未实拍 | `screenshots/print-pdf/*`、`tests.txt` |

### 第 10 项为什么 FAIL

- 任务口径：把窗口设为 140×160（物理、逻辑各一次）后应被钳制到 ≥520×360 逻辑。
- 实测（DPI 144）：物理请求 → 客户区 140×160（93.33×106.67 逻辑）；逻辑请求（210×240 物理）→ 客户区 210×240（140×160 逻辑）。**两者都未被钳制**。
- 同时测得同一产物 `WM_GETMINMAXINFO` 的 `ptMinTrackSize=802×596` 外框（=780×540 客户区=520×360 逻辑），
  且真实鼠标拖拽与 `config` 恢复路径都精确钳到 520×360，最小尺寸下的错误通知首行/末行完整可见。
- 根因：Windows 只在**用户 sizing** 时应用 tracking size；外部程序的 `SetWindowPos` 可绕过，应用未在
  `WM_WINDOWPOSCHANGING` 二次钳制。因此“最小尺寸功能已启用”成立，但“程序化设为 140×160 被钳制”不成立。

## 关键结果

### 最小尺寸（520×360 逻辑）

- 拖拽：1372×1106 → 外框 802×596，客户区 780×540 = **520×360**（`drag-min/metrics.txt`）。
- 恢复：`config` 480×360 → 客户区 780×540（`restore-480x360/`）；`config` 93×107 低于解析下限，整组忽略 → 默认 900×700（`restore-93x107/`）。
- 最小尺寸下通知：首行 `保存失败：拒绝访问。 (os error 5)（写入`、末行 `…；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
  均完整可见（`drag-min/notice.png`）。

### 错误通知常驻

- 链接：`无法打开链接：不支持的链接地址 ftp://example.com`；两图 38,680 B，SHA 同为 `E44EDF8B…FDFB704`。
- 图片：`所选文件不是支持的图片格式；下一步：改用 PNG/JPEG/WebP 等支持的格式`；两图 332,999 B，SHA 同为 `9E0F379E…AF308F55`。
- 图片场景第一版两图仅差 104 px（编辑光标闪烁，bbox 35,114–36,165）；把插入点滚出视口后取图逐字节一致，
  未修改任何像素（细节见 `facts.md` F3）。

### 打印 / PDF 失败回执

- 三类文案均含“下一步”（`i18n.rs:407-420`），3 个单测在 ce038eb 上通过（`screenshots/tests.txt`）。
- 运行时回执未取得：原生「导出 PDF」对话框对只读目标拒绝落盘且保持打开，应用层回执未触发；
  记录为**不可自动采集**，未伪造截图（`screenshots/print-pdf/attempt-*`）。

## 文件

- `README.md`（本文件）、`facts.md`（逐项细节与哈希）、`exe-verify.txt`、`window-min-size.txt`。
- `screenshots/<场景>/`（原始 PNG + walkthrough 日志；`tests.txt`、`print-pdf/` 为单测与打印尝试）。
- `_work/`（驱动脚本与临时文档）、`_home/<场景>/`（每次运行独立的 VEIL_HOME）。
