# UX 修改后证据（spec: polish-veil-ux-craft / after）

本目录只含证据，不修改 tracked 文件、不 commit。采集沿用 `tools/walkthrough.ps1` 与
`VEIL_HOME/APPDATA/LOCALAPPDATA/TEMP` 全隔离；临时文档全部在 `_work/`。

## 产物校验（开始 / 结束）

| 时点 | 字节 | SHA-256 | 提交 |
|---|---|---|---|
| 开始采集（12:38） | 14,006,784 | `C4D84C8AD10C2838CCEC270A80AA91C8840316D64DDBD7FC4C9D522653171FD2` | `3d6adf4` |
| 结束采集（见 `exe-verify.txt` 末行） | 14,006,784 | 同上 | `3d6adf4` |

`exe-verify.txt` 保存了两次原始校验行。工作区本次只改本目录；tracked 改动仅有他人已存在的
`CHANGELOG.md`、`README.md`、`README.zh-CN.md`（未触碰）。

## 显示口径（重要纠正）

- DPI 144（150%），物理屏 `2560x1440`，物理工作区 `0,0,2560,1368`（任务栏占底部 72px）。
- 逻辑屏 `1707x960`，逻辑工作区 `1707x912`。
- baseline 里的“物理屏 1707x960 / 工作区 1707x912”实为 **DPI 未感知** 读数；baseline F0.7/F-a3
  据此刻度推断“默认窗 1050 物理高超出 912”并不成立。真实窗口 1372x1106 物理一直落在
  2560x1368 内。修改后的 `fit_initial_window` 在本屏不触发收缩，只负责居中与夹取。

## 场景总表（前 → 后）

| ID | 场景 / 宽度 | 截图 | 修改前（baseline） | 修改后 | 结论 |
|---|---|---|---|---|---|
| a | 空文档首跑，默认宽（客户区 1350 物理 / 900 逻辑，外框 1372x1106） | `screenshots/a-empty-first-run/01-*`（SAMPLE）、`03-step-empty-first-run-*` | `未命名草稿` / `0 字 · 行 1，列 1`；通知`已新建空白文档` | 同左；窗口居中且完整；无星号 | 通过 |
| b | 900px 窄窗空文档（900 物理 / 600 逻辑，外框 922x1106） | `screenshots/b-narrow-900/02-step-empty-narrow-900-*` | 左右文本完整、通知可见 | 同左；无重叠/裁切 | 通过 |
| c | 深目录长文件名 + 脏态（1350 物理 / 900 逻辑） | `screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-*` | 左文 `文件名 · 第四层 * · 未保存`（双脏标），文件全名可见 | 左文 `文件名 · 第四层 · 未保存`（仅文字脏标），全名仍可见；右 `25 字 · 行 5，列 23` | 通过 |
| d | 保存失败通知（Ctrl+S，目标不可写） | `screenshots/d-save-fail/02-step-save-fail-modal-*`、`03-step-save-fail-notice-*` | 模态 + 遮罩下通知；通知 6s 后可能消失 | 模态同前；错误通知在遮罩下可读，Esc 关模态后仍常驻（错误类不超时） | 通过 |
| e | 未保存关闭对话框（WM_CLOSE） | `screenshots/e-unsaved-close/02-step-unsaved-close-modal-*` | 正文一句`未保存的修改在关闭后将无法恢复。` | 正文改为三行逐按钮说明（保存/不保存/取消）；按钮同前 | 通过 |
| f | 700px 窄窗（700 物理 / 467 逻辑）+ 深目录长名 + 错误通知 | `screenshots/f-narrow-700-longname-notice/02-*`、`statusbar-zoom-x2.png` | 旧代码中段不足即不画通知 | 通知可见（中段省略为`保存失败...S 另存为`）；文件名可见；**` · 未保存` 与右侧统计被挤掉** | 部分不达标（见 R1） |
| f2 | 700 逻辑 px（1050 物理）+ 同场景 | `screenshots/f2-700logical-longname-notice/02-*` | 同上 | 同上：通知与文件名可见，` · 未保存` 仍丢失 | 部分不达标（见 R1） |
| g | 文件菜单展开（空草稿） | `screenshots/g-menu-open/02-step-file-menu-open-*` | 禁用项仅置灰，无原因 | 右侧显示禁用原因；启用项显示快捷键；`导出 HTML/PDF` 主标签与原因均被中段省略 | 部分通过（见 R2） |
| g3 | 编辑菜单展开（空草稿） | `screenshots/g3-edit-menu/02-step-edit-menu-open-*` | 同上 | `撤销`/`重做` 置灰且右侧完整显示`没有可撤销/可重做的操作`；其余快捷键完整 | 通过 |
| h | 查找栏浅色主题焦点环 | `screenshots/h-search-light/02-step-search-light-*` | 焦点仅靠安静底色，无环 | 查询框出现蓝色 1px 焦点环（150% 下按 scale 加粗），清晰可辨 | 通过 |
| i | 查找栏深色主题焦点环 | `screenshots/i-search-dark/02-step-search-dark-*` | 同上 | 深色下环色按 ≥3:1 回退为正文前景色，环清晰；查询框底色 #333A44 | 通过 |
| j | 整窗截图 + 窗口矩形 | `screenshots/j-full-window/02-step-full-window-*`、`window-rect.txt` | 无夹取/居中逻辑；baseline 未记录真实物理矩形 | `593,127,1965,1233`，外框 1372x1106，客户区 1350x1050，DPI 144；工作区 2560x1368 | 通过 |
| k | 崩溃恢复提示（两段式实采） | `screenshots/k-recovery/run2-recovered/02-step-recovery-notice-*`、`statusbar-zoom-x2.png` | 提示文案同样存在，但未实拍 | 恢复对象`recover-me.md`与去向`写回原文件所在目录「recovery」`可见；**恢复时间被中段省略吞掉** | 部分不达标（见 R3） |

补充存档：`g2-edit-menu` 为一次误点（仍展开文件菜单）；`k-recovery/run1-before-kill` 为
自动保存成功、未产生 recovery.dat 的预跑；`k-recovery/run1-dirty` 为注入保存失败后成功
写出 recovery.dat 的正式前态。全部原始日志保留。

## 未解决问题（代码可定位）

- **R1 700px 长名 + 通知时 ` · 未保存` 被丢弃**
  `chrome/paint.rs:317-348` 在通知落位时进入 (a) 分支，把左文压缩为 `identity` 单段
  （`left = self.fit_middle(engine, &identity, room, scale)`），保存态被让位；而
  `chrome/mod.rs:536-539` 的 `core()` 声称“文件身份与保存态都不允许被静默吞掉”。
  既有测试 `chrome/tests.rs:1205`（700px 长名）不含通知；通知测试
  `chrome/tests.rs:1357` 用短路径 `a.md`，两者组合未被覆盖，所以单测全绿但实拍失败。
- **R2 文件菜单禁用原因挤压主标签**
  `chrome/mod.rs:367-382` 的弹窗宽度只量“标签 + 快捷键”，未计入原因；绘制层
  `chrome/paint.rs:489-509` 给原因固定最多 62% 行宽，导致`导出 HTML/PDF`主标签只剩
  `导…`+尾片段、原因退化为`空文档...出的内容`。编辑菜单因原因短而完整。
- **R3 恢复提示丢失“恢复时间”**
  模板 `i18n.rs:333-335` 含`恢复时间 {}`，对象与去向也有；但 1350 物理宽度下
  `notice` 走 `fit_middle`（`chrome/paint.rs:370-381`），中段被省略，时间不可见
  （见 `statusbar-zoom-x2.png`）。对象、去向保留。
- **R4 baseline 屏幕口径错误**
  baseline 的 1707x960/912 是逻辑值；真实物理为 2560x1440/1368。baseline “默认窗超高”结论
  应作废；修改后的窗口实测完整落在工作区内。

## 采集限制

- 空白/低变化帧仍会被 walkthrough 的空白启发式误判，自动降级为置顶屏幕捕获
  （日志 WARN 保留）；这不是产品未绘制。
- 全部场景可自动采集；恢复场景用“文档旁预建 `*.veil-tmp` 目录使保存失败 → 保持脏 →
  2s 后写 recovery.dat → 强杀 → 无文档参数重启”构造，未伪造 recovery 文件。
- 裁剪放大图只由原图底部裁切 2x 生成，原图未改。

## 文件

- `exe-verify.txt`：开始/结束产物校验。
- `window-rect.txt`：窗口矩形 + 工作区（DPI 感知实测）。
- `facts.md`：逐场景“前→后”事实与代码行号、测试行号。
- `screenshots/<场景>/`：原始 PNG + `walkthrough.log` / `stdout.log` / `stderr.log`。
- `_work/`、`_home/`：临时文档、故障注入目录、隔离配置/恢复槽。
