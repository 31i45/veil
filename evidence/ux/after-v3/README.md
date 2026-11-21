# after-v3：最终产物复验（commit 345d8c6）

只写本目录；未改 tracked、未 commit / push；`after/`、`after-v2/` 未动。
采集沿用 `tools/walkthrough.ps1` 与 `VEIL_HOME/APPDATA/LOCALAPPDATA/TEMP` 全隔离。

## 产物校验

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始 14:06 | 14,010,368 | `17D8E0813FF4197E58FD2335867A6118C10941772342B85334650183BECA5EE1` | `345d8c6` |
| 结束 14:08 | 14,010,368 | 同上 | `345d8c6` |

见 `exe-verify.txt`（SHA `C816270D…5B6BBB65`）。

## 结论：after-v2 的 10 项在最终产物上全部复现，无 FAIL

| # | 场景 | 复现结果 | 证据 |
|---|---|---|---|
| 1 | 空文档首跑 | `未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1` | `screenshots/a-empty-first-run/03-*` |
| 2 | 900px 窄窗 | 左右文本与通知完整，无重叠 | `screenshots/b-narrow-900/02-*` |
| 3 | 700px 长名+脏态+错误通知 | 左 `…stuvwxyz.md · 第四层 · 未保存`，右 `24 字 · 行 5，列 14`；通知浮层 5 行完整、无省略号 | `screenshots/f-700-longname-notice/02-*` |
| 4 | 保存失败通知 | 模态同前；通知 2 行浮层完整，Esc 后常驻 | `screenshots/d-save-fail/02-*`、`03-*` |
| 5 | 未保存关闭 | 三行逐按钮说明 + `保存(S)/不保存(C)/取消` | `screenshots/e-unsaved-close/02-*` |
| 6 | 文件菜单 | 动作名完整（`…` 为词条自带）；禁用原因完整`空文档没有可导出的内容` | `screenshots/g-file-menu/02-*` |
| 7 | 查找焦点环·浅色 | 蓝色焦点环可见 | `screenshots/h-search-light/02-*` |
| 8 | 查找焦点环·深色 | 焦点环可见；输入框底色 #333A44 | `screenshots/i-search-dark/02-*` |
| 9 | 整窗与工作区 | 窗口 `593,127,1965,1233`；工作区 `0,0,2560,1368`；无任务栏遮挡 | `screenshots/j-full-window/02-*`、`window-rect.txt` |
| 10 | 崩溃恢复 | `已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 14:08。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`，三字段完整无省略 | `screenshots/k-recovery/run2-recovered/02-*` |

关键文本：

- 700px 通知全文（浮层，5 行）：
  `保存失败：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\after-v3\_work\f\资料归档-2026\第二层目录名称同样很长-用于验证路径截断\third-level-directory-with-a-rather-long-name\第四层\2026-11-21-ux-after-v3-700px-chang-lu-jing-wen-jian-ming-abcdefghijklmnopqrstuvwxyz.md）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
- 菜单：`新建 Ctrl+N`、`打开… Ctrl+O`、`保存 Ctrl+S`、`另存为… Ctrl+Shift+S`、
  `导出 HTML…`/`导出 PDF…`/`打印…`（禁用原因`空文档没有可导出的内容`）。
- 文件名在 700px 下仍按既有策略中段省略（保留头尾）；保存态、通知全文、右侧统计均完整。

## 说明

- v3 只改图片失败文案，上述 UI 行为与此前修复一致；截图中 `e/h/i/g/j` 的哈希与 after-v2 逐字节相同，
  `a/b/d/f/k` 因计时/文件名差异重新生成，结论一致。
- `PrintWindow` 对空白/低变化帧仍会降级为屏幕捕获（日志 WARN）；右/下约 8px 非产品内容为
  DWM 不可见边框，不是 UI 裁切。
- 原始日志与截图均在 `screenshots/<场景>/`；`_work/`、`_home/` 为临时文档和隔离槽。
