# after-v3 事实记录（最终产物 345d8c6）

产物：14,010,368 B，SHA-256
`17D8E0813FF4197E58FD2335867A6118C10941772342B85334650183BECA5EE1`，提交 `345d8c6`。
开始/结束一致（`exe-verify.txt`，336 B，SHA `C816270D9C1AE579E4B45917FF63BA7E9B95E0F5FA17E10D0FC7D7305B6BBB65`）。
DPI 144；物理屏 2560x1440；工作区 `0,0,2560,1368`；逻辑 1707x960/912。
对照基线：`../after-v2/`（commit 7c00765）。v3 只改图片失败文案，UI 修复未回退。

## V1 空文档首跑

截图 `screenshots/a-empty-first-run/03-step-empty-first-run-20261121-140623.png`
（35,513 B，SHA `B15752FACDA0F4790F29770FB310B929A00FAC72DCB601B7A0980FCCCFAA0DE5`）。

- after-v2：`未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`，无星号。
- after-v3：同左，完全复现。SAMPLE 首启图 95,617 B / SHA `C3A86FCB…BACD1B`，与 v2 相同。

## V2 900px 窄窗

截图 `screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-140630.png`
（26,622 B，SHA `7AC9DD7935D791A0771205B269078D88C165043040EE9A6268797B36347959B4`）。
客户区 900 物理 / 600 逻辑，外框 922x1106。

- 文本与通知同 v2：`未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`；无重叠。
  （v2 截图 26,684 B，差异来自光标/计时像素，结论一致。）

## V3 700px 长名 + 脏态 + 错误通知

截图 `screenshots/f-700-longname-notice/02-step-700-longname-notice-20261121-140644.png`
（70,418 B，SHA `26C22C5D83FA7C1A99CAC570AD1F0863E9E8F7E3C84F43BD7470AFA026018A97`）。
客户区 700 物理 / 466.7 逻辑，外框 722x1106；路径长 205；邻接 `*.veil-tmp/` 使自动保存失败。

- 状态栏：`2026-11-21-ux-aft...stuvwxyz.md · 第四层 · 未保存` ｜ `24 字 · 行 5，列 14`。
- 通知浮层 5 行，无 `…`，完整覆盖：对象路径全文 + `可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`。
- after-v2：同一行为（保存态与通知同时可见、无省略）；after-v3 复现，无回归。
- 文件名仍按设计中段省略（保留头尾）；保存态/统计/通知字段未丢。
- 代码沿用 `chrome/paint.rs:256-271`（压缩文件名、保留 `parts.save`、Overlay 兜底）与
  `274-325`（wrap 折行，不省略）；测试 `chrome/tests.rs:1396`。

## V4 保存失败通知

- 模态 `screenshots/d-save-fail/02-step-save-fail-modal-20261121-140715.png`
  （82,816 B，SHA `8F2C4C56D8DF5069923DF0D96F1E9BE50D943B55BA76A7816CAED45557CEAF50`）。
- Esc 后 `screenshots/d-save-fail/03-step-save-fail-notice-20261121-140715.png`
  （49,091 B，SHA `F1F09B4678D1346B189A0F1397320B9A7AAFA8835467F7D22BDFCB2D2D6D6DF5`）。
- 左 `save-fail.md · failsave · 未保存`，右 `19 字 · 行 5，列 12`。
- 通知 2 行浮层完整：`保存失败：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\after-v3\_work\d\failsave\save-fail.md）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`；
  Esc 后常驻。
- 模态标题/原因/下一步/按钮（`重试 (R)/另存为/取消`）与 v2 相同；磁盘原文未变。
- 与 after-v2 结论一致。

## V5 未保存关闭对话框

截图 `screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-140737.png`
（62,713 B，SHA `746B67115FFD03B2E5D2237355B2975A5A6CDE843F145B031B7ADF5851BC5309`，
与 after-v2 逐字节相同）。

- 标题 `是否保存对「unsaved-close.md」的更改？`；正文：
  `「保存」：把本次修改写回原文件。`
  `「不保存」：丢弃本次修改，关闭后无法恢复。`
  `「取消」：返回文档继续编辑。`
- 按钮 `保存 (S)`（主）、`不保存 (C)`（危险红）、`取消`；
  左 `unsaved-close.md · unsaved · 未保存`，右 `20 字 · 行 5，列 12`，无通知。复现。

## V6 文件菜单

截图 `screenshots/g-file-menu/02-step-file-menu-open-20261121-140708.png`
（54,461 B，SHA `EC05CC08359A987785A2125BB218380E2EBF68294D320835AC591769C8ECA165`，
与 after-v2 逐字节相同）。

- 动作名完整：`导出 HTML…`、`导出 PDF…`、`打印…`（`…` 为词条自带，非绘制省略）。
- 快捷键完整：`Ctrl+N / Ctrl+Shift+N / Ctrl+O / Ctrl+P / Ctrl+S / Ctrl+Shift+S / Ctrl+Shift+H`。
- 禁用原因完整：`空文档没有可导出的内容`；弹窗宽度受 420 逻辑 px 上限（`chrome/mod.rs:343,367-389`）。
- 与 after-v2 结论一致，复现。

## V7/V8 查找焦点环（浅 / 深）

- 浅色 `screenshots/h-search-light/02-step-search-light-20261121-140745.png`
  （100,020 B，SHA `7B9EF76F71A1B5A1D466421EDFD9B7256681ADFF91C77A425E9A9619545961A9`，
  与 v2 相同）。
- 深色 `screenshots/i-search-dark/02-step-search-dark-20261121-140752.png`
  （93,615 B，SHA `CCF47D58CED4491B55C5BD1A7017B48794559AD7409A44BC1FA2B25930D90BAE`，
  与 v2 相同）。
- 两者查询框均有 1px×scale 焦点环（深色按 ≥3:1 回退），可见；复现。

## V9 整窗与工作区

截图 `screenshots/j-full-window/02-step-full-window-20261121-140811.png`
（95,617 B，SHA `C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B`）。
测量 `window-rect.txt`（110 B，SHA `EF6A749F8B2E2A3F7CE32B693AA631E48B21D9F3DAF7B27A518731C58471BCE3`）。

- 工作区 `0,0,2560,1368`；窗口 `593,127,1965,1233`，外框 1372x1106，客户区 1350x1050，DPI 144。
- 四边余量 593/127/595/135；底边 1233 < 1368，任务栏不遮挡。复现。

> **更正（保留上行原文）**：外框 1372 − 非客户区 24 = 客户区 **1348**（宽），
> 上行「客户区 1350x1050」中的 1350 为笔误，应读作 1348x1050。

## V10 崩溃恢复提示

构造：`recover-me.md` 旁预建 `*.veil-tmp/` → 自动保存持续失败保持脏 → 写
`_home/k/recovery.dat`（185 B）→ 强杀 → 无文档参数重启。

- 前态 `screenshots/k-recovery/run1-dirty/02-step-dirty-before-kill-20261121-140828.png`
  （53,332 B，SHA `CD2FEEB7E5E4224C9A6CED6B13D5F6278BCB89CDBB3B625242BC0C5AE8F282E2`）。
- 后态 `screenshots/k-recovery/run2-recovered/02-step-recovery-notice-20261121-140838.png`
  （65,261 B，SHA `9E0B7B1A14B98F2A2EDCB11A870EF7F0D883DEC96EBA9E6F05512B18A14243E8`）。
- 逐字：`已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 14:08。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`
  - 对象 `recover-me.md`、时间 `14:08`、去向 `写回原文件所在目录「recovery」` 全部完整，无省略号。
- 状态栏：`recover-me.md · recovery · 未保存` ｜ `23 字 · 行 1，列 1`。
- 与 after-v2 结论一致，复现（时间随本轮变化）。

## 判定

- 10/10 场景 PASS；无 FAIL。
- after-v2 的四项修复（700px 保存态+完整通知、菜单完整、恢复三字段、窗口落区）在最终产物上
  逐项复现。
- 已知非问题：700px 文件名中段省略（保留头尾）；空白帧采集降级；DWM 约 8px 不可见边框。

## 关键文件 SHA-256

| 文件 | 字节 | SHA-256 |
|---|---|---|
| exe-verify.txt | 336 | C816270D9C1AE579E4B45917FF63BA7E9B95E0F5FA17E10D0FC7D7305B6BBB65 |
| window-rect.txt | 110 | EF6A749F8B2E2A3F7CE32B693AA631E48B21D9F3DAF7B27A518731C58471BCE3 |
| a/03-step-empty-first-run | 35,513 | B15752FACDA0F4790F29770FB310B929A00FAC72DCB601B7A0980FCCCFAA0DE5 |
| b/02-step-empty-narrow-900 | 26,622 | 7AC9DD7935D791A0771205B269078D88C165043040EE9A6268797B36347959B4 |
| d/02-step-save-fail-modal | 82,816 | 8F2C4C56D8DF5069923DF0D96F1E9BE50D943B55BA76A7816CAED45557CEAF50 |
| d/03-step-save-fail-notice | 49,091 | F1F09B4678D1346B189A0F1397320B9A7AAFA8835467F7D22BDFCB2D2D6D6DF5 |
| e/02-step-unsaved-close-modal | 62,713 | 746B67115FFD03B2E5D2237355B2975A5A6CDE843F145B031B7ADF5851BC5309 |
| f/02-step-700-longname-notice | 70,418 | 26C22C5D83FA7C1A99CAC570AD1F0863E9E8F7E3C84F43BD7470AFA026018A97 |
| g/02-step-file-menu-open | 54,461 | EC05CC08359A987785A2125BB218380E2EBF68294D320835AC591769C8ECA165 |
| h/02-step-search-light | 100,020 | 7B9EF76F71A1B5A1D466421EDFD9B7256681ADFF91C77A425E9A9619545961A9 |
| i/02-step-search-dark | 93,615 | CCF47D58CED4491B55C5BD1A7017B48794559AD7409A44BC1FA2B25930D90BAE |
| j/02-step-full-window | 95,617 | C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B |
| k/run1-dirty/02-step-dirty-before-kill | 53,332 | CD2FEEB7E5E4224C9A6CED6B13D5F6278BCB89CDBB3B625242BC0C5AE8F282E2 |
| k/run2-recovered/02-step-recovery-notice | 65,261 | 9E0B7B1A14B98F2A2EDCB11A870EF7F0D883DEC96EBA9E6F05512B18A14243E8 |

所有 `stdout.log` 0 B（SHA `E3B0C442…B855`）；各场景原始 `walkthrough.log` 已保留。
