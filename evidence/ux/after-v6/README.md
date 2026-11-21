# after-v6：最终产物 UX 证据全量重绑（commit c19a082）

只写本目录；未改任何 tracked 文件，未 commit / push；`after/`、`after-v2/`…`after-v5/`、`final-v2/`、`final-v3/`
均原样保留。场景采集沿用 `tools/walkthrough.ps1`；极窄窗/最小尺寸、链接与外部文件通知由
`_work/` 内脚本（由 `after-v5/_work` 的 `extreme.ps1`、`min-drag.ps1`、`min-config.ps1`、`link.ps1`
复制后调整路径改写）补充。

## 产物校验

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始 18:29:36 | 14,011,392 | `B921300F9795244C66B5BDC4E977AFD0653B947E0761C5269E6D0CEDF237DE3B` | `c19a082` |
| 结束 19:24:04 | 14,011,392 | 同上 | `c19a082` |

`exe-verify.txt` 记录 START/END 两行。DPI 144（150%），物理屏 2560×1440，工作区 `0,0,2560,1368`；
默认窗口外框 1372×1106 @593,127（客户区 1350×1050 = 900×700 逻辑）。

## 判定：PASS（13/13）

| # | 项 | 判定 | 证据（相对 screenshots/） |
|---|---|---|---|
| 1 | 空文档首跑 | PASS：`未命名草稿 ｜ 已新建空白文档 ｜ 0 字 · 行 1，列 1` | `a-empty-first-run/03-*` |
| 2 | 900px 窄窗 | PASS：左右与通知完整、无重叠 | `b-narrow-900/02-*` |
| 3 | 700px 长名脏态 + 错误通知 | PASS：浮层 6 行完整不省略；`…stuvwxyz.md · 第四层 · 未保存 ｜ 28 字 · 行 4，列 25` | `f-700-longname-notice-r2/02-*` |
| 4 | 保存失败通知 | PASS：模态含原因/下一步/重试(R)/另存为/取消 + 2 行常驻通知 | `d-save-fail/02-*`、`03-*` |
| 5 | 未保存关闭对话框 | PASS：三行逐按钮说明 + `保存(S)/不保存(C)/取消` | `e-unsaved-close/02-*` |
| 6 | 文件菜单 | PASS：动作名/快捷键/禁用原因「空文档没有可导出的内容」完整 | `g-file-menu/02-*` |
| 7 | 浅 / 深焦点环 | PASS：两主题焦点环均清晰 | `h-search-light/02-*`、`i-search-dark/02-*` |
| 8 | 崩溃恢复提示 | PASS：对象/恢复时间/去向三字段完整 | `k-recovery-run2/02-*` |
| 9 | 整窗 / 工作区 | PASS：窗口 1372×1106 @593,127；工作区 2560×1368 | `j-full-window/02-*`、`j-full-window/window-rect.txt` |
| 10 | 最小尺寸 520×360 | PASS：用户拖拽与恢复路径钳到 520×360；140 逻辑尺寸下通知首末行可见 | `m-min-size/drag-min/*`、`logical-140x160/*`、`restore-*` |
| 11 | 链接不支持文案 + 常驻 | PASS：含现状与下一步；t0/t7 逐字节相同 | `l-link-unsupported/notice-t{0,7s}.png` |
| 12 | 链接打开失败文案 + 常驻 | PASS：含原因与下一步；t0/t7 逐字节相同 | `l-link-failed/notice-t{0,7s}.png` |
| 13 | 外部文件被删除 / 暂不可读文案 | PASS：均含现状与下一步 | `n-external-deleted/notice.png`、`n-external-unreadable/notice.png` |

## 文案质检（本轮新增）

- 链接不支持：`无法打开链接：不支持的链接地址 ftp://example.com；下一步：改用 http/https 链接，或复制到浏览器打开`
- 链接打开失败：`无法打开链接：ShellExecuteW 返回 2；下一步：检查系统默认浏览器，或复制链接手动打开`
- 外部被删除：`文件已被外部删除，编辑器中内容仍在，可继续编辑，或用「另存为」保存到新位置`
- 外部暂不可读：`文件暂不可读（可能被外部程序占用）；编辑器中内容仍在，可继续编辑或另存为`

三条错误类文案的 t0/t7 像素一致：链接不支持 43,211 B / SHA `B4E3919C…E9A`；链接打开失败
37,645 B / SHA `BE2822E3…8FA3`。

## 最小尺寸（520×360 逻辑）

- 真实鼠标拖拽：1372×1106 → 外框 802×596，客户区 780×540 = **520×360**，`clamped=True`；
  通知首行 `保存失败：拒绝访问。 (os error 5)（写入` 与末行 `…；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为` 均完整可见。
- 140×160 逻辑请求下（程序化 SetWindowPos，窗口未被钳制）：通知首行与末行仍可见（`logical-140x160/request.png`）。
- `config` 记录 480×360 → 启动钳到 520×360；93×107 低于解析下限被整组忽略 → 默认 900×700。
- 观察（非本轮验收项）：程序化 `SetWindowPos` 仍可把窗口降到 140×160，与 v5 一致（Windows 仅对用户
  sizing 应用 tracking size）；本轮要求的是「用户拖拽被钳制」且已满足。

## 方法与限制

- 截图走 `tools/walkthrough.ps1`（PrintWindow，失败时降级屏幕捕获）；每次运行独立 `VEIL_HOME`（`_home/<场景>/`），
  未触碰真实用户配置。
- 链接打开失败用白名单内 `mailto:%zz@example.com`（ShellExecuteW 返回 2，原生失败路径）；先 Ctrl+End 把插入点
  移到长文档末尾、再滚回顶部，使光标不在视口内，从而 t0/t7 逐字节一致。未改注册表、未临时劫持系统协议处理器；
  早期用 `tel:`/超长 `mailto:` 的尝试会打开 Edge 或留下 tooltip，均已删除、不作为证据。
- 只读文档在运行期 `attrib +R`、结束后恢复；未改仓库 tracked 文件。

## 文件

- `README.md`（本文件）、`facts.md`（逐项细节与哈希）、`exe-verify.txt`。
- `screenshots/<场景>/`（原始 PNG + walkthrough 日志）；`_work/`（驱动脚本与临时文档）；`_home/<场景>/`（独立 VEIL_HOME）。
