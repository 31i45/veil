# after-v7：最终产物 UX 证据重绑（commit 0d229eb）

只写本目录；未改任何 tracked 文件，未 commit / push。`after/` … `after-v6/`、`final/` … `final-v4/`
原样保留。场景采集沿用 `tools/walkthrough.ps1`；最小尺寸、链接、外部文件探针复制自
`after-v6/_work` 并改路径；配置未保存与 Mermaid 快照失败为本轮新增探针（`_work/` 内）。

## 产物校验

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始 20:15:36 | 14,012,928 | `8DF551F8D363C73DE1F35887042BEC7775E5AB8D1FFEDB50F365AE7E9B456763` | `0d229eb` |
| 结束（见 `exe-verify.txt` END 行） | 同上 | 同上 | `0d229eb` |

DPI 144（150%），物理屏 2560×1440，工作区 `0,0,2560,1368`；默认窗口外框 1372×1106 @593,127
（客户区 1350×1050 = 900×700 逻辑）。

## 判定：PASS（10/10 + 6/6 文案）

| # | 项 | 判定 | 证据（相对 screenshots/） |
|---|---|---|---|
| 1 | 空文档首跑 | PASS | `a-empty-first-run/03-*` |
| 2 | 900px 窄窗 | PASS | `b-narrow-900/02-*` |
| 3 | 700px 长名脏态 + 错误通知 | PASS | `f-700-longname-notice/02-*` |
| 4 | 保存失败通知 | PASS | `d-save-fail/02-*`、`03-*` |
| 5 | 未保存关闭对话框 | PASS | `e-unsaved-close/02-*` |
| 6 | 文件菜单 | PASS | `g-file-menu/02-*` |
| 7 | 浅 / 深焦点环 | PASS | `h-search-light/02-*`、`i-search-dark/02-*` |
| 8 | 崩溃恢复提示 | PASS | `k-recovery-run2/02-*` |
| 9 | 整窗 / 工作区 | PASS | `j-full-window/02-*`、`window-rect.txt` |
| 10 | 最小尺寸 520×360 | PASS | `m-min-size/*` |
| 11 | 链接不支持 / 打开失败 | PASS | `l-link-unsupported/notice-t{0,7s}.png`、`l-link-failed/notice-t{0,7s}.png` |
| 12 | 外部文件删除 / 暂不可读 | PASS | `n-external-{deleted,unreadable}/notice.png` |
| 13 | 配置未保存 | PASS | `q-config-not-saved/{notice.png,metrics.txt}` |
| 14 | Mermaid 快照保存失败 | PASS | `q-mermaid-snap/{notice.png,menu.png,metrics.txt}` |

## 文案质检（本轮新增代表项）

- 链接不支持 / 打开失败：原因 + 下一步；t0 与 t7s 逐字节相同（错误通知常驻）。
- 外部被删除 / 暂不可读：现状（内容仍在）+ 下一步（继续编辑 / 另存为）。
- 配置未保存：`配置未保存：拒绝访问。 (os error 5)（写入 …\config.toml）；下一步：可检查配置目录的写入权限或磁盘空间后重试`。
- Mermaid 快照失败：`Mermaid 快照保存失败，未修改原文：…（创建 …\mermaid-snap.assets）；下一步：可检查目标目录是否可写后重新插入快照`。
- 全量失败 Key 由 `all_failure_notices_offer_next_step` 单测穷举（`_work/i18n-test.log`，1 passed）。

## 与 v6 的差异

`_work/compare.txt`：`e-unsaved-close/02`、`g-file-menu/02`、`h/i` 搜索、`j` 整窗与 v6 逐字节相同；
`d-save-fail/02,03`、`f-700/02`、`k` 恢复场景的差异均在文案/状态栏/时间区域，无布局回归。

## 方法与限制

- 截图走 `tools/walkthrough.ps1`（PrintWindow，失败降级屏幕捕获）；每场景独立 `VEIL_HOME`。
- 只读文档运行期 `attrib +R`、结束恢复；未改 tracked 文件。
- 观察（非 FAIL）：程序化 `SetWindowPos` 拉到 140×160 时通知被裁切，与 v6 相同；用户拖拽与配置恢复均精确停在 520×360，该尺寸通知全文可见。

## 文件

- `README.md`（本文件）、`facts.md`（逐项细节与哈希）、`exe-verify.txt`（START/END）。
- `screenshots/<场景>/`；`_work/`（驱动脚本、输入文档、对比与单测日志）；`_home/<场景>/`（独立 VEIL_HOME）。
