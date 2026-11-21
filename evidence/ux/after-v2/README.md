# UX 残留复验（after-v2 / commit 7c00765）

只写本目录；未改 tracked、未 commit / push。采集沿用 `tools/walkthrough.ps1` 与
`VEIL_HOME/APPDATA/LOCALAPPDATA/TEMP` 全隔离，临时文档都在 `_work/`。

## 产物校验（开始 / 结束）

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始 13:12 | 14,010,368 | `25FEA5F3A65430211F05D9BFA2EC0667672C88D31AAC66F90DE4BC56ACEE2B67` | `7c007651` |
| 结束 13:17 | 14,010,368 | 同上 | `7c007651` |

原始两行见 `exe-verify.txt`（SHA `E9CB1BBA…8983D86`）。工作区 tracked 改动仍只有他人既有的
`CHANGELOG.md`、`README.md`、`README.zh-CN.md`。

## 复验结论

| # | v1 残留 | v2 实测 | 判定 | 证据 |
|---|---|---|---|---|
| 1 | 700px + 长名 + 错误通知：` · 未保存` 被挤掉、通知中段省略 | 左文 `…stuvwxyz.md · 第四层 · 未保存`；通知改为状态栏上方浮层、完整折行，无省略号；右文 `24 字 · 行 5，列 14` 仍在 | **PASS**（附注 1） | `screenshots/f-700-longname-notice/02-*`、`statusbar-notice-zoom-x2.png`；测试 `chrome/tests.rs:1396` |
| 2 | 文件菜单动作名/原因被中段省略 | `导出 HTML…`、`导出 PDF…`、`打印…` 动作名完整（`…` 是词条自带）；禁用原因完整显示`空文档没有可导出的内容`；弹窗宽度受 `POPUP_MAX_W=420` 逻辑 px 夹取 | **PASS** | `screenshots/g-file-menu/02-*`；测试 `chrome/tests.rs:1432,1524` |
| 3 | 恢复提示的“恢复时间”被中段省略 | 单行浮层完整：`已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 13:13。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`，对象/时间/去向齐全，无省略号 | **PASS** | `screenshots/k-recovery/run2-recovered/02-*`、`statusbar-notice-zoom-x2.png`；测试 `chrome/tests.rs:1473` |
| 4 | 其余场景回归 | 空文档首跑、900px 窄窗、保存失败、未保存关闭、浅/深焦点环、整窗矩形均与 v1 一致或更好 | **PASS** | 下方回归表 |

未出现 FAIL。附注 1：700px 下**文件名**仍按既有策略中段省略（保留头尾，
`2026-11-21-ux-aft...stuvwxyz.md`）；保存态、通知全文、右侧统计均未被吞。若把“文件名全文”
视为硬性字段，则仅此项受 700px 物理宽度限制。

## 判定细节

### 1) 700px 长名 + 脏态 + 错误通知

- 窗口：客户区 700 物理 / 466.7 逻辑，外框 722x1106，DPI 144。
- 草稿路径长度 205，邻接 `*.veil-tmp/` 目录使自动保存失败；错误通知不超时。
- 状态栏（放大图逐字）：
  `2026-11-21-ux-aft...stuvwxyz.md · 第四层 · 未保存` ｜ `24 字 · 行 5，列 14`
- 通知浮层（5 行，完整）：
  `保存失败：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\after-v2\_work\f\资料归档-2026\第二层目录名称同样很长-用于验证路径截断\third-level-directory-with-a-rath
  er-long-name\第四层\2026-11-21-ux-after-v2-700px-chang-lu-jing-wen-jian-ming-abcdefghijklmnopqrstuvwxyz.md）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
- 代码：`layout_status_notice` 先压缩文件名，保留 `parts.save`（`chrome/paint.rs:256-262`）；
  中段/右文区都容不下时走 `NoticeSlot::Overlay`（`chrome/paint.rs:267-271`）；
  浮层 `wrap_notice` 折行、不省略（`chrome/paint.rs:274-325`）。
- 测试：`notice_keeps_unsaved_state_at_700px_long_name`（`chrome/tests.rs:1396`）。
- 说明：使用 v1 同一物理宽度口径（700 物理），比 700 逻辑更窄，属更严格测试。

### 2) 文件菜单

- 空草稿 + Ctrl+N 后点“文件”。
- 启用项快捷键完整：`Ctrl+N / Ctrl+Shift+N / Ctrl+O / Ctrl+P / Ctrl+S / Ctrl+Shift+S /
  Ctrl+Shift+H`。
- 禁用项（`导出 HTML…`、`导出 PDF…`、`打印…`）原因完整`空文档没有可导出的内容`；
  动作名保留词条自带的 U+2026（`i18n.rs:56-58`），非绘制省略。
- 代码：弹窗宽度把禁用原因计入（`chrome/mod.rs:367-389`），上限 420 逻辑 px
  （`chrome/mod.rs:343`）；绘制时先保动作名，只有提示可省略（`chrome/paint.rs:617-632`）。
- 测试：`disabled_reason_reserves_width_so_action_name_is_whole`（`chrome/tests.rs:1432`）、
  `disabled_menu_item_paints_reason_text`（`chrome/tests.rs:1524`）。

### 3) 崩溃恢复

- 构造：`recover-me.md` 旁预建 `*.veil-tmp/` → 输入后自动保存持续失败、保持脏 →
  写 `_home/k/recovery.dat`（184 B）→ 强杀 → 无文档参数重启，恢复优先加载。
- 浮层逐字：`已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 13:13。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`
- 对象 `recover-me.md`、时间 `13:13`、去向 `写回原文件所在目录「recovery」` 均完整；
  状态栏 `recover-me.md · recovery · 未保存` ｜ `23 字 · 行 1，列 1`。
- 代码/测试：提示组装 `app.rs:893-914`；浮层折行 `chrome/paint.rs:274-325`；
  `recovery_notice_overlay_keeps_object_time_and_destination`（`chrome/tests.rs:1473`）。

## 回归重采

| 场景 | 截图 | 结果 |
|---|---|---|
| 空文档首跑（1350/900 逻辑） | `screenshots/a-empty-first-run/03-step-empty-first-run-*` | `未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`，无星号 |
| 900px 窄窗（900 物理/600 逻辑） | `screenshots/b-narrow-900/02-step-empty-narrow-900-*` | 左右文本与通知完整，无重叠 |
| 保存失败通知（Ctrl+S） | `screenshots/d-save-fail/02-*`、`03-*` | 模态文案同 v1；通知改为 2 行浮层、完整可见；Esc 后常驻 |
| 未保存关闭（WM_CLOSE） | `screenshots/e-unsaved-close/02-*` | 三行逐按钮说明、`保存(S)/不保存(C)/取消` 与 v1 一致 |
| 查找焦点环·浅色 | `screenshots/h-search-light/02-*` | 蓝色焦点环可见 |
| 查找焦点环·深色 | `screenshots/i-search-dark/02-*` | 环可见；输入框底色 #333A44 |
| 整窗 + 矩形 | `screenshots/j-full-window/02-*`、`window-rect.txt` | 窗口 `593,127,1965,1233`；外框 1372x1106；工作区 `0,0,2560,1368`；无任务栏遮挡 |

空白/低变化帧仍可能被 walkthrough 的空白检测降级为屏幕捕获（日志 WARN），属采集启发式，
不是产品未绘制。右侧/底部若出现约 8px 的其它窗口内容，是 DWM 不可见缩放边框，不是裁切。

## 文件

- `README.md` / `facts.md`
- `exe-verify.txt`、`window-rect.txt`
- `screenshots/<场景>/`（原始 PNG + walkthrough/stdout/stderr 日志）
- `_work/`、`_home/`（临时文档、故障注入、隔离配置与恢复槽）
