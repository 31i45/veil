# UX 修改后事实记录（前 → 后，逐条可溯源）

产物：`target/release/veil.exe`，14,006,784 B，SHA-256
`C4D84C8AD10C2838CCEC270A80AA91C8840316D64DDBD7FC4C9D522653171FD2`，提交 `3d6adf4`。
采集开始/结束均校验一致（`exe-verify.txt`，SHA `AFA619DE…D64335`）。
DPI 144（150%）；物理屏 2560x1440；物理工作区 0,0,2560,1368；逻辑 1707x960 / 1707x912。
截图/日志原始文件在 `screenshots/<场景>/`。baseline 证据路径以 `../baseline/` 引用。

> 代码行号均取 HEAD `3d6adf4`（源码未改，只有 CHANGELOG/README 有他人未提交改动）。
> “前”的行号取 baseline 记录的 `5cdf015`（同 `97d3a60` 源码树）。

## F0 通则与关键改动

- F0.1 状态栏分段与降级：`chrome/mod.rs:542-560`（Full→NoSource→Core）、
  `status_parts` `chrome/mod.rs:562-591`；脏态只保留 ` · 未保存`（`i18n.rs:138`），
  不再叠加 ` *`。测试 `chrome/tests.rs:1188-1199`。
  - 前：` · 未保存` 前还有 ` *`（baseline F-c1）；后：仅文字。
- F0.2 通知落位四档：中段→压缩左文→占用右文→状态栏上方浮层，保证不静默丢弃：
  `chrome/paint.rs:313-348`、`350-411`。
  - 前：中段不足（<`space::XL*scale`）直接不画（baseline `chrome/paint.rs:239-257`）。
- F0.3 错误类通知不参与 6s 消退：`chrome/mod.rs:630-645`、
  `app.rs:1699-1716`；测试 `app_tests.rs:17322,17394`。
- F0.4 菜单禁用项右侧显示原因（而非只置灰）：`chrome/paint.rs:473-530`、
  `disabled_reason` `chrome/mod.rs:784`；测试 `chrome/tests.rs:1395`、
  `app_tests.rs:13062,13080`。
  - 前：禁用项仍是灰色标签 + 快捷键；后：禁用原因替换快捷键。
- F0.5 焦点环 ≥3:1：`view/search_view.rs:92-105`、
  `focus_ring_color` `view/modal.rs:549-557`；测试 `app_tests.rs:17838`。
  - 前：仅 `focus_bg` 安静底色，无环。
- F0.6 首窗夹取/居中：`app.rs:1808-1815,1819-1859,1869-1914`，接入
  `app.rs:1925-1975`；测试 `app_tests.rs:17533,17555`。
  - 前：直接 900x700 逻辑、无位置计算（baseline F0.7）。
- F0.7 恢复提示对象/时间/去向：`app.rs:876-922`（模板组装 893-914），
  文案 `i18n.rs:333-337`；恢复写盘 `doc_state.rs:306-351`，
  测试 `app_tests.rs:9872`（锁定目标下自动保存持续失败仍写恢复）、`9716`（重启恢复）。
- F0.8 未保存关闭模态正文逐按钮解释：`i18n.rs:295-297`，
  模态构造 `doc_state.rs:934-944`，关闭执行 `doc_state.rs:1105`。
- F0.9 保存失败：通知 `i18n.rs:253`，手工失败弹窗 `doc_state.rs:194-212`，
  按钮 `doc_state.rs:1289`；原子写仍为同目录 `*.veil-tmp` + rename。

## F-a 空文档首跑（默认宽）

截图：`screenshots/a-empty-first-run/01-w1350-p0-20261121-123730.png`
（SAMPLE，95,617 B，SHA `C3A86FCB…BACD1B`）、`03-step-empty-first-run-20261121-123730.png`
（空草稿，35,006 B，SHA `DACBF4F5…92FAE`）。客户区 1350 物理 / 900 逻辑，外框 1372x1106。

- 前（`../baseline/screenshots/a-empty-first-run/02-step-empty-first-run-20261121-103151.png`）：
  `未命名草稿` / `0 字 · 行 1，列 1`，通知`已新建空白文档`。
- 后：文本相同；无星号；窗口完整显示，标题、菜单、状态栏齐备。
- 下一步：光标在 1 行 1 列，通知 6s 后消退（Info 类仍走 TTL）。

## F-b 900px 窄窗空文档

截图：`screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-123902.png`
（26,459 B，SHA `578D94F6…F16D94`）；客户区 900 物理 / 600 逻辑，外框 922x1106。

- 前（`../baseline/screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-103227.png`）：
  左右完整、通知可见。
- 后：相同；无重叠、无裁切。`PrintWindow` 空白启发式仍降级屏幕捕获（日志 WARN），
  非产品未绘制。

## F-c 深目录长文件名 + 脏态（默认宽）

截图：`screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-20261121-123917.png`
（61,426 B，SHA `6826B68D…0FBA5F6`）；客户区 1350 物理 / 900 逻辑。
临时文档路径长度 210。

- 前（`../baseline/screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-20261121-103258.png`）：
  `2026-11-21-ux-baseline-…-abcdefghijklmnop.md · 第四层 * · 未保存`。
- 后：`2026-11-21-ux-after-…-abcdefghijklmnop.md · 第四层 · 未保存`；
  文件全名可见、无中段省略；右 `25 字 · 行 5，列 23` 完整；无通知、无模态。
- 代码：脏标单一来源 `chrome/mod.rs:562-591`；路径只显示“文件名 · 直接父目录”
  `chrome/mod.rs:492-508`（深路径仍不显示，属既有设计）。

## F-d 保存失败（Ctrl+S，目标不可写）

注入：文档旁预建 `save-fail.md.veil-tmp/` 目录，使临时文件写入失败（F0.9）。

截图：模态 `screenshots/d-save-fail/02-step-save-fail-modal-20261121-123942.png`
（74,931 B，SHA `AA4DE594…8D3F4946`）；Esc 后通知
`03-step-save-fail-notice-20261121-123942.png`（54,965 B，SHA `B817A6D1…39CD0790`）。
客户区 1350 物理 / 900 逻辑。

- 前（baseline F-d2/d3）：模态文案相同；通知在遮罩下偏暗；错误通知会按 TTL 消退。
- 后：
  - 左文 `save-fail.md · failsave · 未保存`（无星号）；右 `19 字 · 行 5，列 12`。
  - 通知：`保存失败：拒绝访问。 (os error 5) （写入 D:\VS…）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`
    （路径中段省略，其余完整）；在模态遮罩下仍可读。
  - 模态：`保存失败`（红标题/叹号）+ `save-fail.md · failsave` +
    `原因：拒绝访问。 (os error 5) （写入 …\after\_work\d\failsave\save-fail.md)` +
    `下一步：可重试本次保存，或用「另存为」写入其他位置；当前修改仍保留在编辑器中。`
    按钮 `重试 (R)`（主）、`另存为`、`取消`。
  - Esc 关模态后通知仍为保存失败且常驻（错误类不超时，F0.3），自动化重试再失败会重设同一文案；
    磁盘原文未变（stderr 3 行，`stderr.log` SHA `C3CEB5DF…BF78C2`）。

## F-e 未保存关闭对话框（WM_CLOSE）

截图：`screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-124008.png`
（62,713 B，SHA `746B6711…BC5309`）；客户区 1350 物理 / 900 逻辑。

- 前（baseline F-e2）：正文单句`未保存的修改在关闭后将无法恢复。`
- 后：标题 `是否保存对「unsaved-close.md」的更改？`；
  正文三行：
  `「保存」：把本次修改写回原文件。`
  `「不保存」：丢弃本次修改，关闭后无法恢复。`
  `「取消」：返回文档继续编辑。`
  按钮 `保存 (S)`（主）、`不保存 (C)`（危险红字）、`取消`。
  左文 `unsaved-close.md · unsaved · 未保存`（无星号），右 `20 字 · 行 5，列 12`，无通知。
- 代码：`i18n.rs:295-297`；按钮顺序 `doc_state.rs:934-944`。

## F-f 700px 窄窗 + 长路径脏态 + 通知

截图：`screenshots/f-narrow-700-longname-notice/02-step-narrow700-longname-notice-20261121-124029.png`
（42,894 B，SHA `75CCDFC7…A85B7AD8`），客户区 700 物理 / 467 逻辑。
放大条：`statusbar-zoom-x2.png`（16,325 B，SHA `503EF3EF…6D5493`）。
通知用自动保存失败触发（错误类常驻），未弹模态。

- 前：旧绘制层在中段不足时不画通知（F0.2 前）。
- 后：通知**可见**，但被中段省略为 `保存失败...S 另存为`；
  左文 `2026-11-21-ux-after-7...opqrstuvwxyz.md · 第四层`；
  **没有 ` · 未保存`，右侧字数/行列也整段让位**。
- 结论：满足“通知不被吞”“文件名可见”，不满足“未保存可见”（R1）。
- 代码：`chrome/paint.rs:317-348`；`core()` 的承诺见 `chrome/mod.rs:536-539`。
  测试空档：`chrome/tests.rs:1205`（700px 长名，无通知）、`1357`（通知，短路径 `a.md`）。

## F-f2 700 逻辑 px（1050 物理）+ 同场景

截图：`screenshots/f2-700logical-longname-notice/02-step-700logical-longname-notice-20261121-124118.png`
（50,529 B，SHA `031AF616…6217892`），客户区 1050 物理 / 700 逻辑。

- 与 F-f 同：文件名变得更能辨认（前缀更长），通知与文件名可见，` · 未保存` 仍丢失，
  右侧统计让位。说明 R1 不是“物理 700 太窄”的偶发，而是通知压缩路径的固定行为。

## F-g 文件菜单

截图：`screenshots/g-menu-open/02-step-file-menu-open-20261121-124143.png`
（51,452 B，SHA `88B3C5A1…3E2C5237`）。

- 前：禁用项置灰，无原因。
- 后：启用项显示快捷键（`Ctrl+N/O/P/S/Shift+S/Shift+H` 等）；禁用项右侧显示原因：
  `保存` 未禁用；`导出 HTML/PDF`、`打印` 置灰并显示 `空文档...出的内容`，
  但 `导出 HTML/PDF` 的主标签也被挤到只剩 `导…`+尾片段（R2）。
- 代码：原因列最多占 62% `chrome/paint.rs:489-509`；弹窗宽度不计原因
  `chrome/mod.rs:367-382`。

## F-g3 编辑菜单

截图：`screenshots/g3-edit-menu/02-step-edit-menu-open-20261121-124521.png`
（53,735 B，SHA `42392E43…1F0D5B92`）。

- 后：`撤销`/`重做` 置灰，右侧完整显示`没有可撤销的操作`/`没有可重做的操作`；
  `剪切/复制/粘贴/全选/加粗/斜体/链接/查找/替换/插入图片` 的快捷键完整可见。
- 说明：禁用项的快捷键位被原因替代（F0.4），启用项不受影响。
- 存档：`g2-edit-menu` 为误点（仍展开文件菜单），保留原始文件，不作为结论。

## F-h / F-i 查找栏焦点环（浅色 / 深色）

- 浅色：`screenshots/h-search-light/02-step-search-light-20261121-124206.png`
  （100,020 B，SHA `7B9EF76F…545961A9`）。
- 深色：`screenshots/i-search-dark/02-step-search-dark-20261121-124225.png`
  （93,615 B，SHA `CCF47D58…30D90BAE`），`mode="dark"` 来自隔离 `config.toml`。
- 前：`../baseline` 无查找截图；代码上焦点仅 `focus_bg`，无环
  （baseline `view/search_view.rs` 注释“安静底色表达，不再画 accent 下划线”）。
- 后：查询框四周出现 1px×scale 焦点环；浅色为 accent 蓝，深色按对比度回退正文色
  （≥3:1，`view/modal.rs:549-557`）。深色查询框底色实测 #333A44，环清晰。
- 测试：`app_tests.rs:17838`。

## F-j 整窗与工作区

截图：`screenshots/j-full-window/02-step-full-window-20261121-124251.png`
（95,617 B，SHA `C3A86FCB…BACD1B`）。
测量：`window-rect.txt`（SHA `EF6A749F…58471BCE3`）。

- DPI 感知实测：
  - 工作区 `0,0,2560,1368`（2560x1368）。
  - 窗口 `593,127,1965,1233`，外框 1372x1106，客户区 1350x1050，DPI 144。
  - 边距：左 593、上 127、右 595、下 135（到工作区）；底边 1233 < 1368，任务栏（1368–1440）
    不遮挡。
- 前：baseline 未记录矩形；其 `1707x912` 为逻辑值，被误当物理值，故“默认窗超屏”结论作废（R4）。
- 代码：`fit_initial_window` `app.rs:1869-1914`；接入 `app.rs:1964-1969`；
  测试 `app_tests.rs:17533,17555`。

## F-k 崩溃恢复提示

构造（未伪造 recovery）：文档旁预建 `recover-me.md.veil-tmp/` → 输入后自动保存持续失败、
保持脏 → 2s 后写 `_home/k2/recovery.dat`（184 B）→ walkthrough 强杀 → 无文档参数重启，
recovery 优先加载。

- 前态：`screenshots/k-recovery/run1-dirty/02-step-dirty-before-kill-20261121-124411.png`
  （58,330 B，SHA `C501365B…A0CF4CD`）。
- 后态：`screenshots/k-recovery/run2-recovered/02-step-recovery-notice-20261121-124426.png`
  （58,488 B，SHA `8F4A4468…CBE0115`）；放大条
  `screenshots/k-recovery/run2-recovered/statusbar-zoom-x2.png`
  （25,745 B，SHA `77569F5B…79F8F2F2`）。
- 实际渲染：`recover-me.md · recovery · 未保存` +
  `已从崩溃恢复「recover-me.md」的未保存...S 保存（写回原文件所在目录「recovery」）`+
  `23 字 · 行 1，列 1`。
  - 对象：可见（`recover-me.md`）。
  - 保存去向：可见（`写回原文件所在目录「recovery」`）。
  - 时间：模板含`恢复时间 {}`（`i18n.rs:333-334`），但中段省略把“内容；恢复时间 12:44。按 Ctrl+”
    整段吞掉，**时间不可见**（R3）。
- 代码：恢复提示组装 `app.rs:893-914`；写盘 `doc_state.rs:306-351`；
  测试 `app_tests.rs:9872,9716`。

## 自动采集结论

- 5 个沿用场景 + 700px 通知 + 文件/编辑菜单 + 浅/深查找环 + 整窗 + 崩溃恢复：全部自动采集成功。
- 不可自动采集：无。
- 恢复场景依赖“保存失败保持脏”的构造；不注入失败时自动保存会在 2s 先把内容落盘并清脏，
  recovery.dat 不会产生（预跑 `k-recovery/run1-before-kill` 已保留说明）。

## 关键文件 SHA-256（完整清单见各目录）

| 文件 | 字节 | SHA-256 |
|---|---|---|
| exe-verify.txt | 336 | AFA619DE9B4CA8DEDA338DB10379A62C5C74477DD7B7A74C7437E09A66D64335 |
| window-rect.txt | 110 | EF6A749F8B2E2A3F7CE32B693AA631E48B21D9F3DAF7B27A518731C58471BCE3 |
| a/03-step-empty-first-run | 35,006 | DACBF4F56C3BDF38F3B1F2771765BB7A0DEF99C876080531CE4327D9DB492FAE |
| b/02-step-empty-narrow-900 | 26,459 | 578D94F67C81ADFCBE277D0A495173C8485A5ED27C8E5964F3E481F845F16D94 |
| c/02-step-dirty-deep-long-name | 61,426 | 6826B68DBEAD033BA5DA764E35603E3C11138F5907584ADB25DACD09C0FBA5F6 |
| d/02-step-save-fail-modal | 74,931 | AA4DE594D2C84079B6C833616C797FFFF762019D32ABF9F24D63343C8D3F4946 |
| d/03-step-save-fail-notice | 54,965 | B817A6D15D6929354DBDDFAB9E343928B8CA3411384CFC9C156AF1DD39CD0790 |
| e/02-step-unsaved-close-modal | 62,713 | 746B67115FFD03B2E5D2237355B2975A5A6CDE843F145B031B7ADF5851BC5309 |
| f/02-step-narrow700-longname-notice | 42,894 | 75CCDFC72A85951F120206378C455B4ECE52F00D69C07A7CB1F96116A85B7AD8 |
| f/statusbar-zoom-x2 | 16,325 | 503EF3EFFB33A5F0B69FB3705F8A1276F636472D0135CC37557E4998866D5493 |
| f2/02-step-700logical-longname-notice | 50,529 | 031AF6165470BD3BC4B5597010BC6F6DBEB30FB58CB70C821087541F06217892 |
| g/02-step-file-menu-open | 51,452 | 88B3C5A184D2D06416A82781F43F6A113DFD4B077A09B18D8520018D3E2C5237 |
| g3/02-step-edit-menu-open | 53,735 | 42392E438979A673E7DE82BB3DFBA4E18B260903E555FE9F64CF6E441F0D5B92 |
| h/02-step-search-light | 100,020 | 7B9EF76F71A1B5A1D466421EDFD9B7256681ADFF91C77A425E9A9619545961A9 |
| i/02-step-search-dark | 93,615 | CCF47D58CED4491B55C5BD1A7017B48794559AD7409A44BC1FA2B25930D90BAE |
| j/02-step-full-window | 95,617 | C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B |
| k/run1-dirty/02-step-dirty-before-kill | 58,330 | C501365B72277845ADD5C1F15D6960883FC04FCCAAE7B4A5F6E058C52A0CF4CD |
| k/run2-recovered/02-step-recovery-notice | 58,488 | 8F4A4468E53D8828FB53D488D751CB47859B7DC45DCF00F2711AB66A9CBE0115 |
| k/run2-recovered/statusbar-zoom-x2 | 25,745 | 77569F5B67B7044980F74FFF8585E352CF31D6E21FC4586AB0C94B6079F8F2F2 |

所有 `stdout.log` 为 0 字节（SHA `E3B0C442…B855`）。
