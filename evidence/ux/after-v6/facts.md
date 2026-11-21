# after-v6 事实记录（最终产物 c19a082）

产物 `target/release/veil.exe`：14,011,392 B，SHA-256
`B921300F9795244C66B5BDC4E977AFD0653B947E0761C5269E6D0CEDF237DE3B`，提交 `c19a082`。
开始 18:29:36 / 结束 19:24:04 两次校验一致（`exe-verify.txt`，2 行）。
DPI 144（150%）；物理屏 2560×1440；工作区 `0,0,2560,1368`；默认窗口外框 1372×1106 @593,127
（客户区 1350×1050 = 900×700 逻辑）。

相对 v5（ce038eb）的产品改动仅为 i18n 文案：链接不支持/打开失败、脚注/锚点、外部文件删除/不可读
补「下一步」与内容仍在说明；最小尺寸逻辑未变。

## F1 十个场景

| # | 场景 | 结果 | 证据与哈希（相对 screenshots/） |
|---|---|---|---|
| 1 | 空文档首跑 | `未命名草稿 ｜ 已新建空白文档 ｜ 0 字 · 行 1，列 1` | `a-empty-first-run/03-step-empty-first-run-20261121-183125.png`（26,437 B，`EACEDD84…C27137`） |
| 2 | 900px 窄窗 | 左右与通知完整、无重叠 | `b-narrow-900/02-step-empty-narrow-900-20261121-183131.png`（23,921 B，`3DFBF561…10C5BE`） |
| 3 | 700px 长名脏态 + 错误通知 | 浮层 6 行完整；状态栏 `…stuvwxyz.md · 第四层 · 未保存 ｜ 28 字 · 行 4，列 25` | `f-700-longname-notice-r2/02-step-700-longname-notice-20261121-184620.png`（41,518 B，`0F877729…1B31D6`） |
| 4 | 保存失败通知 | 模态含原因/下一步 + 2 行常驻通知 | `d-save-fail/02-step-save-fail-modal-20261121-183137.png`（82,157 B，`F43C5454…E37C8BF`）、`03-…png`（48,173 B，`D1058093…CA9F77`） |
| 5 | 未保存关闭 | 三行逐按钮说明 + `保存(S)/不保存(C)/取消` | `e-unsaved-close/02-step-unsaved-close-modal-20261121-183148.png`（61,061 B，`104D4BBF…F84F33`，与 v5 同 SHA） |
| 6 | 文件菜单 | 动作名/快捷键/禁用原因完整 | `g-file-menu/02-step-file-menu-open-20261121-183205.png`（54,461 B，`EC05CC08…C8ECA165`，与 v5 同 SHA） |
| 7 | 浅色焦点环 | 可见（蓝环） | `h-search-light/02-step-search-light-20261121-183212.png`（99,824 B，`1AA0D99C…80096A75`，与 v5 同 SHA） |
| 8 | 深色焦点环 | 可见 | `i-search-dark/02-step-search-dark-20261121-183219.png`（93,416 B，`E8B5636A…AABB0E9F`，与 v5 同 SHA） |
| 9 | 整窗 / 工作区 | 1372×1106 @593,127；工作区 2560×1368 | `j-full-window/02-…png`（95,617 B，`C3A86FCB…DBACD1B`）、`window-rect.txt` |
| 10 | 最小尺寸 520×360 | 用户拖拽/恢复钳到 520×360；140 逻辑尺寸通知首末行可见 | `m-min-size/*` |

第 10 项细节：

- `[A]` 真实鼠标从右下角边框拖拽：客户区 780×540 物理 = **520×360 逻辑**，`clamped=True`；
  `drag-min/metrics.txt`，`drag-min/notice.png`（63,770 B，`F035930A…BFF62154`）。
- `[B]` 140×160 逻辑请求（程序化 SetWindowPos）→ 客户区 210×240 物理 = 140×160 逻辑，**未被钳制**；
  `logical-140x160/request.png`（12,250 B，`938B3B27…F48655`）中通知首行 `保存失败：拒绝…` 与末行 `另存为`
  仍可见（中间行受宽度限制）。
- `[C]` 物理 140×160 请求 → 客户区 140×160 物理（93.33×106.67 逻辑），同样未被 SetWindowPos 钳制；
  `physical-140x160/request.png`。
- `[D]` `config` 记录 480×360 → 启动客户区 780×540 = 520×360；`restore-480x360/metrics.txt`。
- `[E]` `config` 93×107 低于解析下限 → 整组忽略 → 默认 900×700；`restore-93x107/metrics.txt`。
- 判定：本轮验收的两条子项（140 逻辑尺寸通知首末行可见、用户拖拽被钳制）均 PASS；程序化 SetWindowPos
  可绕过 min track 属 Windows 行为，与 v5 相同，作为观察记录，不列为本轮 FAIL。

## F2 链接与外部文件文案质检

- 链接不支持（`ftp://example.com`，Ctrl+点击）：`无法打开链接：不支持的链接地址 ftp://example.com；下一步：改用 http/https 链接，或复制到浏览器打开`。
  `l-link-unsupported/notice-t0.png`、`notice-t7s.png` 均 43,211 B，SHA `B4E3919C3E09C13CCE2F747487001EED026DE5B4737C777AC5237AFAAC619E9A`，`byte_identical=True`。
- 链接打开失败（`mailto:%zz@example.com`，Ctrl+点击）：`无法打开链接：ShellExecuteW 返回 2；下一步：检查系统默认浏览器，或复制链接手动打开`。
  `l-link-failed/notice-t0.png`、`notice-t7s.png` 均 37,645 B，SHA `BE2822E3A0010571D4702EE7A39F8A8EB60B2CE1A7B6105BC6CB2B2856878FA3`，`byte_identical=True`；
  `metrics.txt` 记录 `foreground_after_click=veil`、`printwindow_t0/t7=True`。
  方法：先 Ctrl+End 把插入点移到长文档末尾，再滚轮回滚到顶部（插入点不在视口，应用不绘制光标），
  然后 Ctrl+点击首行链接；未改注册表、未劫持协议处理器。
- 外部文件被删除：`文件已被外部删除，编辑器中内容仍在，可继续编辑，或用「另存为」保存到新位置`；
  `n-external-deleted/notice.png`（36,297 B，`36AACF8E…14E171`）。
- 外部文件暂不可读（独占写+删共享句柄占用）：`文件暂不可读（可能被外部程序占用）；编辑器中内容仍在，可继续编辑或另存为`；
  `n-external-unreadable/notice.png`（36,313 B，`4C21846C…AE7380A`）。

## F3 与 v5 的像素差异

`_work/compare.txt` 记录同场景 v5/v6 逐张对比：除链接/外部文件场景外，内容区差异仅为插入光标与滚动条
等 100–300 px 级差异（例如 `d-save-fail/02` 212 px、`f-700/02` 212 px）；`e-unsaved-close/02`、`g-file-menu/02`、
`h/i` 搜索、`j` 整窗与 v5 逐字节相同。文案改动未造成布局回归。

## F4 方法与限制

- 全部截图走 `tools/walkthrough.ps1`（PrintWindow，失败时降级屏幕捕获）；每次运行独立 `VEIL_HOME`。
- 极窄窗/最小尺寸、链接/外部文件通知使用 `_work/` 内一次性驱动脚本（由 after-v5 同名脚本复制并改路径）。
- 只读文档在运行期 `attrib +R`、结束后恢复为 Archive；未改仓库 tracked 文件。
- 早期链接失败尝试（`tel:`、超长 `mailto:`）会打开 Edge 或残留 tooltip，已删除，未作为证据。
- 打印/PDF 运行时回执未在本轮重采（不在 after-v6 验收项内）；文案单测 `screenshots/print-pdf/tests.txt` 保留。
