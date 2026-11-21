# after-v2 事实记录（v1 残留 → v2 结果）

产物：`target/release/veil.exe`，14,010,368 B，SHA-256
`25FEA5F3A65430211F05D9BFA2EC0667672C88D31AAC66F90DE4BC56ACEE2B67`，提交 `7c007651`。
开始/结束两次校验一致（`exe-verify.txt`，336 B，SHA `E9CB1BBAAF832AA2C00676F7C44C4A7534F23CEEBC072C611FA521A336983D86`）。
DPI 144（150%）；物理屏 2560x1440；物理工作区 `0,0,2560,1368`；逻辑 1707x960 / 1707x912。
代码行号取 HEAD `7c007651`。v1 证据在 `../after/`，baseline 在 `../baseline/`。

## V0 结论总览

| 项 | v1 残留 | v2 结果 | 判定 |
|---|---|---|---|
| 700px 长名+脏+错误通知 | “ · 未保存”被挤掉，通知中段省略 | 保存态可见；通知浮层完整折行；无省略号 | PASS（文件名仍按设计中段省略，见 N1） |
| 文件菜单 | 导出项动作名与原因被省略 | 动作名完整（`…` 为词条自带）、原因完整；宽度受 420 逻辑 px 上限 | PASS |
| 崩溃恢复 | “恢复时间”被中段省略 | 对象/时间/去向三字段单行完整可见 | PASS |
| 其余 7 项回归 | — | 与 v1 一致，无回归 | PASS |

无 FAIL。

## V1 700px + 深目录长名 + 脏态 + 错误通知

截图：`screenshots/f-700-longname-notice/02-step-700-longname-notice-20261121-131248.png`
（70,241 B，SHA `063E826C13AD8FB9633D5765C5D8C2876E646700881E3D2F03D4C1C4BE9E78A7`），
放大底部 `statusbar-notice-zoom-x2.png`（58,311 B，SHA `C682F0BBC4C4FA8086F48FB205B43B4BD5F9817BE1ABC6CB5ED9025545B686FB`）。
客户区 700 物理 / 466.7 逻辑；外框 722x1106；路径长度 205。

- v1（`../after/screenshots/f-narrow-700-longname-notice/02-*`）：左文只剩
  `2026-11-21-ux-after-7...opqrstuvwxyz.md · 第四层`，` · 未保存` 与右侧统计消失；
  通知压进状态栏并省略为`保存失败...S 另存为`。
- v2 状态栏逐字：
  `2026-11-21-ux-aft...stuvwxyz.md · 第四层 · 未保存` ｜ `24 字 · 行 5，列 14`。
- v2 通知：状态栏上方浮层，5 行完整，无 `…`。全文：
  `保存失败：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\after-v2\_work\f\资料归档-2026\第二层目录名称同样很长-用于验证路径截断\third-level-directory-with-a-rath` +
  `er-long-name\第四层\2026-11-21-ux-after-v2-700px-chang-lu-jing-wen-jian-ming-abcdefghijklmnopqrstuvwxyz.md）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
- 代码：`layout_status_notice` 压缩文件名时用 `save_w` 预留保存态，拼回
  `format!("{identity}{}", parts.save)`（`chrome/paint.rs:256-262`）；中段、右文区都放不下时
  返回 `NoticeSlot::Overlay`（`267-271`）；浮层 `wrap_notice` 折行、逐行绘制、无
  `ellipsize_middle`（`274-325`）。
- 测试：`notice_keeps_unsaved_state_at_700px_long_name`（`chrome/tests.rs:1396`）；
  700px 布局本身 `status_layout_700px_long_path_keeps_filename_and_unsaved`（`1205`）。
- N1：文件名仍按既有策略中段省略（保留头尾）。保存态、通知全文、右侧统计均未丢；
  700 物理宽度下若要文件名全文只能折行状态栏，本轮未要求。

## V2 文件菜单（空草稿）

截图：`screenshots/g-file-menu/02-step-file-menu-open-20261121-131313.png`
（54,461 B，SHA `EC05CC08359A987785A2125BB218380E2EBF68294D320835AC591769C8ECA165`）。

- v1（`../after/screenshots/g-menu-open/02-*`）：`导出 HTML/PDF` 主标签被挤成
  `导…`+尾片段，原因显示为`空文档...出的内容`。
- v2：
  - 启用项动作名 + 快捷键完整：`新建 Ctrl+N`、`新建窗口 Ctrl+Shift+N`、`打开… Ctrl+O`、
    `快速打开 Ctrl+P`、`保存 Ctrl+S`、`另存为… Ctrl+Shift+S`、`版本历史 Ctrl+Shift+H`。
  - 禁用项动作名完整：`导出 HTML…`、`导出 PDF…`、`打印…`（`…` 为词条自带，
    `i18n.rs:56-58`）；原因完整：`空文档没有可导出的内容`。
- 代码：宽度计算把 `DisabledEmptyDocument` 的原因宽计入（`chrome/mod.rs:367-389`）；
  上限 `POPUP_MAX_W = 420.0` 逻辑（`chrome/mod.rs:343`）；绘制优先保动作名，
  仅提示列允许省略（`chrome/paint.rs:617-632`）。
- 测试：`disabled_reason_reserves_width_so_action_name_is_whole`（`chrome/tests.rs:1432`）、
  `disabled_menu_item_paints_reason_text`（`1524`）、
  `disabled_menu_actions_always_explain_why`（`504`）。

## V3 崩溃恢复提示

- 构造（未伪造 recovery 文件）：`recover-me.md` 旁预建 `recover-me.md.veil-tmp/` →
  输入后自动保存持续失败、保持脏 → 2s 写 `_home/k/recovery.dat`（184 B）→ walkthrough
  强杀 → 无文档参数重启，恢复优先加载。
- 前态：`screenshots/k-recovery/run1-dirty/02-step-dirty-before-kill-20261121-131336.png`
  （53,287 B，SHA `AE88EF192F3EDEF961ED95A0401C19F18F3A4A77ADD9767DC36B3D0065E93B88`）。
- 后态：`screenshots/k-recovery/run2-recovered/02-step-recovery-notice-20261121-131351.png`
  （65,278 B，SHA `4142DEB2C5E1AD3601B4C885067D37FECFE8FAA6233D4AF41C22E1115F75E9BF`），
  放大 `statusbar-notice-zoom-x2.png`（37,044 B，SHA `2A289E57A47BB144AF77FD448B0C43514BE7E49C08E06D839A00DBC899EF6594`）。
- v1（`../after/screenshots/k-recovery/run2-recovered/02-*`）：中段省略吞掉
  “内容；恢复时间 12:44。按 Ctrl+”，时间不可见。
- v2 逐字（单行浮层，无省略号）：
  `已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 13:13。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`
  - 对象：`recover-me.md`；时间：`13:13`；去向：`写回原文件所在目录「recovery」`。
- 状态栏：`recover-me.md · recovery · 未保存` ｜ `23 字 · 行 1，列 1`。
- 代码/测试：提示组装 `app.rs:893-914`；文案 `i18n.rs:333-337`；
  浮层折行 `chrome/paint.rs:274-325`；
  `recovery_notice_overlay_keeps_object_time_and_destination`（`chrome/tests.rs:1473`）。

## V4 回归：空文档首跑

截图：`screenshots/a-empty-first-run/03-step-empty-first-run-20261121-131221.png`
（35,503 B，SHA `29F9C78B7574AA574B64A92CC2C0E5CA280A412F2C788E0245B7AA1CAF7CD248`）。
SAMPLE 首启 `01/02` 均为 95,617 B / SHA `C3A86FCB…BACD1B`。

- `未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`，无星号。
- 与 v1 相同；右侧/底部约 8px 其它窗口内容为 DWM 不可见边框，不是产品裁切。

## V5 回归：900px 窄窗

截图：`screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-131233.png`
（26,684 B，SHA `9BD3B669F2CB57BDEBBE9E9EA5CE10ECA739B49BE12B69F6C3C07657567A33E1`）。
客户区 900 物理 / 600 逻辑，外框 922x1106。

- `未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`；左右与通知完整，无重叠。

## V6 回归：保存失败通知

- 模态：`screenshots/d-save-fail/02-step-save-fail-modal-20261121-131409.png`
  （82,788 B，SHA `973892762A35F9B8592791849EC0E20CB6046C593ECAE9EC1AB0A349A45CB233`）。
- Esc 后：`screenshots/d-save-fail/03-step-save-fail-notice-20261121-131409.png`
  （49,076 B，SHA `38D5B7BA0D7AECAAB369942846D4EB9973BD1569386F8137CC8561B4E8FD546C`）。
- 左 `save-fail.md · failsave · 未保存`，右 `19 字 · 行 5，列 12`。
- 通知由 v1 的状态栏内联改为 2 行浮层，完整显示：
  `保存失败：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\after-v2\_work\d\failsave\save-fail.md）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
  （路径全文，无省略号）；Esc 关模态后常驻。
- 模态标题/正文/按钮与 v1 一致：`保存失败`、原因、下一步、`重试 (R)/另存为/取消`。
- 磁盘原文未变；`stderr.log` 3 行（561 B，SHA `D4C66BC7…037B5E`）。

## V7 回归：未保存关闭对话框

截图：`screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-131435.png`
（62,713 B，SHA `746B67115FFD03B2E5D2237355B2975A5A6CDE843F145B031B7ADF5851BC5309`，与 v1 同图同哈希）。

- 标题 `是否保存对「unsaved-close.md」的更改？`；正文三行：
  `「保存」：把本次修改写回原文件。`
  `「不保存」：丢弃本次修改，关闭后无法恢复。`
  `「取消」：返回文档继续编辑。`
- 按钮 `保存 (S)`（主）、`不保存 (C)`（危险红）、`取消`；
  左 `unsaved-close.md · unsaved · 未保存`，右 `20 字 · 行 5，列 12`，无通知。

## V8 回归：查找焦点环（浅 / 深）

- 浅色：`screenshots/h-search-light/02-step-search-light-20261121-131452.png`
  （100,020 B，SHA `7B9EF76F71A1B5A1D466421EDFD9B7256681ADFF91C77A425E9A9619545961A9`，与 v1 同图）。
- 深色：`screenshots/i-search-dark/02-step-search-dark-20261121-131507.png`
  （93,615 B，SHA `CCF47D58CED4491B55C5BD1A7017B48794559AD7409A44BC1FA2B25930D90BAE`，与 v1 同图）。
- 查询框四周 1px×scale 焦点环；浅色为 accent 蓝，深色按 ≥3:1 回退正文色
  （`view/modal.rs:549-557`）；深色输入框底色 #333A44。
- 测试：`app_tests.rs:17838`。

## V9 回归：整窗与工作区

截图：`screenshots/j-full-window/02-step-full-window-20261121-131519.png`
（95,617 B，SHA `C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B`）。
测量：`window-rect.txt`（110 B，SHA `EF6A749F8B2E2A3F7CE32B693AA631E48B21D9F3DAF7B27A518731C58471BCE3`）。

- DPI 感知实测工作区 `0,0,2560,1368`；窗口 `593,127,1965,1233`，外框 1372x1106，
  客户区 1350x1050，DPI 144。
- 四边余量：左 593、上 127、右 595、下 135；底边 1233 < 1368，任务栏（1368–1440）不遮挡。
- 与 v1 一致；窗口尺寸未变（900x700 逻辑默认值在本屏不需要收缩）。

## 采集限制

- 空白/低变化帧可能被 walkthrough 空白启发式降级为置顶屏幕捕获（日志 WARN），属采集启发式。
- 700px 采用 v1 同一物理宽度（700 物理 = 466.7 逻辑），比 700 逻辑更严格；
  未另测 700 逻辑，因为更宽只会更容易满足。
- 右侧/底部约 8px 的非产品内容来自 DWM 不可见缩放边框/屏幕捕获区域，不是 UI 裁切。

## 关键文件 SHA-256

| 文件（相对本目录） | 字节 | SHA-256 |
|---|---|---|
| exe-verify.txt | 336 | E9CB1BBAAF832AA2C00676F7C44C4A7534F23CEEBC072C611FA521A336983D86 |
| window-rect.txt | 110 | EF6A749F8B2E2A3F7CE32B693AA631E48B21D9F3DAF7B27A518731C58471BCE3 |
| a/03-step-empty-first-run | 35,503 | 29F9C78B7574AA574B64A92CC2C0E5CA280A412F2C788E0245B7AA1CAF7CD248 |
| b/02-step-empty-narrow-900 | 26,684 | 9BD3B669F2CB57BDEBBE9E9EA5CE10ECA739B49BE12B69F6C3C07657567A33E1 |
| d/02-step-save-fail-modal | 82,788 | 973892762A35F9B8592791849EC0E20CB6046C593ECAE9EC1AB0A349A45CB233 |
| d/03-step-save-fail-notice | 49,076 | 38D5B7BA0D7AECAAB369942846D4EB9973BD1569386F8137CC8561B4E8FD546C |
| e/02-step-unsaved-close-modal | 62,713 | 746B67115FFD03B2E5D2237355B2975A5A6CDE843F145B031B7ADF5851BC5309 |
| f/02-step-700-longname-notice | 70,241 | 063E826C13AD8FB9633D5765C5D8C2876E646700881E3D2F03D4C1C4BE9E78A7 |
| f/statusbar-notice-zoom-x2 | 58,311 | C682F0BBC4C4FA8086F48FB205B43B4BD5F9817BE1ABC6CB5ED9025545B686FB |
| g/02-step-file-menu-open | 54,461 | EC05CC08359A987785A2125BB218380E2EBF68294D320835AC591769C8ECA165 |
| h/02-step-search-light | 100,020 | 7B9EF76F71A1B5A1D466421EDFD9B7256681ADFF91C77A425E9A9619545961A9 |
| i/02-step-search-dark | 93,615 | CCF47D58CED4491B55C5BD1A7017B48794559AD7409A44BC1FA2B25930D90BAE |
| j/02-step-full-window | 95,617 | C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B |
| k/run1-dirty/02-step-dirty-before-kill | 53,287 | AE88EF192F3EDEF961ED95A0401C19F18F3A4A77ADD9767DC36B3D0065E93B88 |
| k/run2-recovered/02-step-recovery-notice | 65,278 | 4142DEB2C5E1AD3601B4C885067D37FECFE8FAA6233D4AF41C22E1115F75E9BF |
| k/run2-recovered/statusbar-notice-zoom-x2 | 37,044 | 2A289E57A47BB144AF77FD448B0C43514BE7E49C08E06D839A00DBC899EF6594 |

关键日志：`f/walkthrough.log` SHA `92C0A9D3…DBF4098`；`g/walkthrough.log` SHA
`CE4C92B1…BBA45DB`；`k/run2-recovered/walkthrough.log` SHA `E21D79C0…742F659B`。
所有 `stdout.log` 为 0 B（SHA `E3B0C442…B855`）。
