# UX 基线事实记录（逐场景 + 代码行号）

## Corrigendum（2026-11-21）

- 原文「物理屏 1707x960（工作区 1707x912）」为**逻辑像素**值（DPI 未感知读数）误记为**物理像素**，属口径混淆；
  150% DPI 下 960 逻辑 = 1440 物理，1707 逻辑 ≈ 2560 物理。
- 真实物理分辨率为 **2560×1440**；实测物理工作区为 **2560×1368**（任务栏占底部 72 px）。
- 证据不可改写：以下原文（含 F0.7、F-a3 等依赖 912 高度的判断）均按原样保留，本小节只作更正说明，
  不改动任何截图、日志及其 SHA-256。依据为 `../after/README.md` 的 DPI 感知实测（`window-rect.txt`）。

被测产物：`target/release/veil.exe`，13,998,592 B，SHA-256
`58C9AD0DECFD614DA73098A006AC0773B2515CD49C56684B3F1AEC5FD0D4AB75`。
系统 DPI 144（150%）；物理屏 1707x960（工作区 1707x912）。
采集：`tools/walkthrough.ps1`；隔离 `VEIL_HOME/APPDATA/LOCALAPPDATA/TEMP`。
截图原始文件与 `walkthrough.log` 均在 `screenshots/<场景>/`，本文件不改写它们。

> 代码行号说明：`app.rs`、`i18n.rs`、`view/modal.rs`、`app_tests.rs` 在本轮采集期间被
> 其他工作并发修改（未提交，非本任务所为）。为避免行号漂移，本文件的这三处行号取
> `git HEAD`（`5cdf015`，其源码树与生成该 exe 的 `97d3a60` 一致）；`chrome/*`、`paint.rs`、
> `document/*` 未被并发修改，行号与工作区一致。

## F0 通用事实（代码依据）

- F0.1 状态栏几何：菜单高 30、状态栏高 26（逻辑 px），绘制时乘 scale。
  `crates/veil-ui/src/chrome/mod.rs:25-26`；状态栏绘制 `chrome/paint.rs:187-238`。
- F0.2 状态栏文本拼装：左 = `path_label` + 脏标 + 外部变更 + 源码模式 + 保存态；
  右 = `字数 字 · 行 L，列 C`。
  `chrome/mod.rs:517-552`；脏标 `" *"`（`i18n.rs:128`）与保存态 `" · 未保存"`（`i18n.rs:133`）
  同时出现，属双重提示；干净且本会话保存过时显示 `" · 已保存 HH:MM"`（`i18n.rs:134`）。
- F0.3 路径标签只拼 `文件名 · 直接父目录`，不显示完整深路径；无路径显示`未命名草稿`。
  `chrome/mod.rs:490-508`。
- F0.4 左文超宽时中段省略（head…尾），右文先固定测宽；`chrome/paint.rs:204-220`、
  `ellipsize_middle` `chrome/paint.rs:27-53`。
- F0.5 通知（notice）只在左右文本之间的中段空白绘制，且可用宽 ≥ `space::XL*scale`
  才画，否则不画；`chrome/paint.rs:239-257`。通知 TTL 6 秒，`app.rs:55`、
  `expire_notice` `app.rs:1647-1665`。
- F0.6 模态覆盖整窗（含菜单栏/状态栏）且最后绘制；`paint.rs:1962-1969`、
  `paint_modal` `paint.rs:2052-2077`、`view/modal.rs:335-396`（遮罩→卡片→标题→正文→按钮），
  按钮绘制 `view/modal.rs:446+`。
- F0.7 窗口默认尺寸 900x700 逻辑（`app.rs:1787-1788`）。150% 下为 1350x1050 物理，
  高于本机工作区高度 912；场景 a 的 pilot 目录记录了默认高度下的截图异常（见 F-a3）。
- F0.8 关窗请求统一进入 `WindowEvent::CloseRequested`（`app.rs:1893-1896`）→
  `confirm_close`（`document/doc_state.rs:1087-1089`）→ `begin_unsaved`（`doc_state.rs:918-955`）。
- F0.9 保存为同目录 `*.veil-tmp` + rename 原子写（`crates/veil/src/lib.rs:645-658`）；
  目标侧同名 `*.veil-tmp` 被占成目录时，写临时文件即失败（场景 d 的故障注入方式）。
- F0.10 自动保存静默期 2 秒（`veil-ui/src/autosave.rs:12`），失败只写状态栏通知，
  手工保存失败才弹模态（`doc_state.rs:190-206`）。

## F-a 空文档首跑（默认宽度）

截图（客户区 1350 物理 = 默认 900 逻辑；窗口矩形 1372x896）：

- `screenshots/a-empty-first-run/01-w1350-p0-20261121-103151.png`
  SHA-256 `900B9DBD1FB28BFC2C53F2AB2049423BB446F00624DCA4BA00D40E2A07CF31A9`
- `screenshots/a-empty-first-run/02-step-empty-first-run-20261121-103151.png`
  SHA-256 `17558BCF21339A96D7FF1C0943D5F144B726978C029E6B2FE07B192A210A2C52`

事实：

- F-a1 隔离首启（无文档参数）进入 SAMPLE 欢迎页；01 图状态栏左 `未命名草稿`，
  右 `181 字 · 行 1，列 1`，无脏标，无通知。SAMPLE 来源 `crates/veil/src/lib.rs:23,152-153`。
- F-a2 `Ctrl+N` 后为空白未命名草稿；02 图左 `未命名草稿`，右 `0 字 · 行 1，列 1`；
  中段通知 `已新建空白文档` 可见（`i18n.rs:297`）；无模态。左右文本完整、无重叠。
- F-a3 默认高度 700 逻辑时窗口 1372x1106 物理，超出 912 的屏幕工作区；
  `screenshots/a-empty-default-900x700-pilot/02-step-empty-first-run-20261121-103042.png`
  （36,849 B，SHA `4584268E…1B101`）为屏幕捕获降级，窗口下沿/左缘混入屏外与窗口边框内容。
  该目录仅作“默认高度不适配 150% 小屏”的旁证，评分请用 F-a1/F-a2 的完整窗口图。
- F-a4 采集脚本日志：`screenshots/a-empty-first-run/walkthrough.log`
  （SHA `01C540A73FD5471BD0EB44D51EEE1182C348A43A3909E06DB4067878CEFA0C68`）；
  `PrintWindow` 对 02 判为空白帧并降级为屏幕捕获（日志 WARN），属空白文档启发式误判。
- F-a5 下一步动作：光标停在空文档第 1 行第 1 列，可直接输入或 Ctrl+O；通知 6 秒后自动消退。

## F-b 窄窗口约 900 px 的空文档

截图（客户区 900 物理 = 600 逻辑；窗口矩形 922x896）：

- `screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-103227.png`
  SHA-256 `3074B6E76C4AF76D950804151B30B1EE29CD729F1F4B9F269BB258DEDE924BFD`

事实：

- F-b1 状态栏左 `未命名草稿`、右 `0 字 · 行 1，列 1`，均完整；中段 `已新建空白文档` 可见；
  菜单栏`文件/编辑/视图`与状态栏无重叠、无裁切。
- F-b2 同 a 的空白帧降级：`screenshots/b-narrow-900/walkthrough.log`
  （SHA `62972BDF…9B12`）有 `PrintWindow 无效 … 降级为置顶屏幕捕获`。
- F-b3 下一步动作：同 F-a5；窗口 900 物理宽仍可容纳左右状态文本与完整通知。

## F-c 深目录 + 长文件名 + 脏态

临时文档（路径长度 216）：

`_work/c/资料归档-2026/第二层目录名称同样很长-用于验证路径截断/third-level-directory-with-a-rather-long-name/第四层/2026-11-21-ux-baseline-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md`

截图（客户区 1350 物理）：

- `screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-20261121-103258.png`
  SHA-256 `FC44B0518595C94D7657223BA7AAE80A454EEA1BB5E0177917744DCC070B1D48`

事实：

- F-c1 状态栏左完整：`2026-11-21-ux-baseline-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md · 第四层 * · 未保存`。
  文件名未触发中段省略（`ellipsize_middle` 未生效）；父目录只显示`第四层`，深路径不可见
  （F0.3）。右 `29 字 · 行 5，列 23` 完整；无通知、无模态。
- F-c2 标题栏为 `* veil — <文件名>`，脏标前置于整个标题（`i18n.rs:121-123`、
  `doc_state.rs:51-53,65-67`），不是紧贴文件名。
- F-c3 脏态来源：`type:` 步骤以剪贴板粘贴写入 `未保存的脏文本 dirty baseline`；截图时刻仍脏。
  截图后约 2 秒自动保存成功（`autosave.rs:12`；`doc_state.rs:289-296`），临时文件现已含该行，
  且 `_home/c/snapshots` 产生两份快照；截图本身仍代表脏态时刻。
- F-c4 日志 `screenshots/c-deep-path-dirty/walkthrough.log`
  （SHA `517EC99A…13B24FB`）；`PrintWindow` 两次判空白并降级屏幕捕获（空白启发式，非未绘制）。
- F-c5 下一步动作：状态栏已提示`未保存`；用户可 Ctrl+S 落盘，或继续输入。

## F-d 保存失败（Ctrl+S，目标不可写）

故障注入：文档旁预建同名目录 `save-fail.md.veil-tmp/`，使原子写的临时文件路径不可写
（F0.9）；未改 ACL、未改只读属性。

截图（客户区 1350 物理）：

- 模态：`screenshots/d-save-fail/02-step-save-fail-modal-20261121-103337.png`
  SHA-256 `D4623901BE7F9AAE9849422B8BB21CEBA0C614A5A88C666624153924A9D4528B`
- 通知（Esc 关模态后）：`screenshots/d-save-fail/03-step-save-fail-notice-20261121-103337.png`
  SHA-256 `258B049C2840CC1D325B35D57614CB6E35B7BF77BDFB7F22231BAB5D94542F71`

事实：

- F-d1 状态栏左 `save-fail.md · failsave * · 未保存`，右 `19 字 · 行 5，列 12`，均完整。
- F-d2 状态栏通知可见（在模态遮罩下偏暗）：
  `保存失败：拒绝访问。 (os error 5) （写入 D:\V…）；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为`。
  因中段空间不足，路径被中段省略（`chrome/paint.rs:239-257`）；文案来源 `i18n.rs:246`。
- F-d3 模态正文（错误态，红色感叹号 + 红标题`保存失败`）：
  1) `save-fail.md · failsave`
  2) `原因：拒绝访问。 (os error 5) （写入 D:\VSCode\Veil\evidence\ux\baseline\_work\d\failsave\save-fail.md)`
  3) `下一步：可重试本次保存，或用「另存为」写入其他位置；当前修改仍保留在编辑器中。`
  按钮顺序：`重试 (R)`（主按钮，蓝色）、`另存为`、`取消`。
  文案 `i18n.rs:247-249`；按钮构造 `doc_state.rs:196-204`；绘制 `view/modal.rs:335-396,446+`。
- F-d4 `postkey:ESC` 关闭模态后，03 图无模态，通知仍为保存失败文案：
  自动保存重试再次失败会重新设置同一通知（`doc_state.rs:190-193`），TTL 6 秒（`app.rs:55`）。
  03 图状态栏左侧/右侧与 02 相同，仍为脏态。
- F-d5 保存失败日志（stderr）3 行，见 `screenshots/d-save-fail/stderr.log`
  （SHA `66C69AE37092A9EE553AD090147A46AC7FB5DA4D9B0B5FE18C4019319ED9DF76`）；
  磁盘原文未变（仅 `# 保存失败场景` 两段），证明失败未留下半截文件。
- F-d6 下一步动作已给出：模态`重试`写同一路径、`另存为`换位置、`取消`保留修改；
  通知同时提示 Ctrl+S / Ctrl+Shift+S。

## F-e 未保存关闭对话框（脏态关窗）

截图（客户区 1350 物理）：

- `screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-103410.png`
  SHA-256 `8E00958E77B17B397686C90BB2016A4B290F89D7C75D54DC8C8D23BA1542784C`

事实：

- F-e1 以 `close` 步骤发送 `WM_CLOSE`（等价关窗请求，`app.rs:1893-1896`），窗口未关闭，
  而是弹出应用内模态，说明脏态关窗被正确拦截。
- F-e2 模态正文：标题 `是否保存对「unsaved-close.md」的更改？`，正文
  `未保存的修改在关闭后将无法恢复。`，按钮 `保存 (S)`（主按钮）、`不保存 (C)`（危险色红字）、
  `取消`。文案 `i18n.rs:285-286`、按钮 `doc_state.rs:934-944`、绘制 `view/modal.rs:335-396,446+`。
- F-e3 状态栏左 `unsaved-close.md · unsaved * · 未保存`、右 `20 字 · 行 5，列 12`，均完整；
  中段无通知（被模态遮罩覆盖前即为空）。标题 `* veil — unsaved-close.md`。
- F-e4 未选择按钮即结束采集（脚本强制结束自启进程）；临时文档未写入输入文本，
  `_home/e/recovery.dat` 保留崩溃恢复副本。选择`不保存`会清恢复副本并退出
  （`doc_state.rs:960-963,1076-1078`）；点`取消`则提示`已取消：当前文档仍有未保存的修改`
  （`doc_state.rs:1080-1082`、`i18n.rs:294`）。
- F-e5 下一步动作：三按钮均明确；Alt+F4 未单独实拍，但同样走 CloseRequested 路径。

## 不可自动采集

- 无。5 个要求的场景均由 `tools/walkthrough.ps1` 成功采集。
- 未覆盖的组合（非失败，仅范围外）：900px 宽 + 超长文件名 + 脏态的省略效果；
  Alt+F4 路径。人工步骤见 `README.md` 的“自动采集能力结论”。

## 截图/日志清单（SHA-256）

| 文件（相对本目录） | 字节 | SHA-256 |
|---|---|---|
| screenshots/a-empty-first-run/01-w1350-p0-20261121-103151.png | 88,013 | 900B9DBD1FB28BFC2C53F2AB2049423BB446F00624DCA4BA00D40E2A07CF31A9 |
| screenshots/a-empty-first-run/02-step-empty-first-run-20261121-103151.png | 28,260 | 17558BCF21339A96D7FF1C0943D5F144B726978C029E6B2FE07B192A210A2C52 |
| screenshots/a-empty-first-run/walkthrough.log | 1,302 | 01C540A73FD5471BD0EB44D51EEE1182C348A43A3909E06DB4067878CEFA0C68 |
| screenshots/a-empty-default-900x700-pilot/01-w1350-p0-20261121-103042.png | 95,617 | C3A86FCB236EC68D87B078FBD6DF30D58F99B3238F42689CCCDC0A957DBACD1B |
| screenshots/a-empty-default-900x700-pilot/02-step-empty-first-run-20261121-103042.png | 36,849 | 4584268EE8FC766928138EBD689FCB5F4254302AECB8829B1EEE173C14B1B101 |
| screenshots/b-narrow-900/01-w900-p0-20261121-103227.png | 81,953 | F97D12A1BF67757B49999205CD8C47390CFF9CF54276BAA06EB23746EC21060E |
| screenshots/b-narrow-900/02-step-empty-narrow-900-20261121-103227.png | 26,743 | 3074B6E76C4AF76D950804151B30B1EE29CD729F1F4B9F269BB258DEDE924BFD |
| screenshots/b-narrow-900/walkthrough.log | 1,272 | 62972BDFBE6CE2E95E4E26A5E4DD3AA54655CB30B324A3650DAC786CAEBE9B12 |
| screenshots/c-deep-path-dirty/01-w1350-p0-20261121-103258.png | 48,015 | 168EF44ED3B221FEB4149FFF64F5370F304BCEBEC8033EF39F36AC5CB6A5D213 |
| screenshots/c-deep-path-dirty/02-step-dirty-deep-long-name-20261121-103258.png | 54,826 | FC44B0518595C94D7657223BA7AAE80A454EEA1BB5E0177917744DCC070B1D48 |
| screenshots/c-deep-path-dirty/walkthrough.log | 1,825 | 517EC99A8A96BE578D9039C984143046149FBC993338BF13FE357239413B24FB |
| screenshots/d-save-fail/01-w1350-p0-20261121-103337.png | 35,186 | EE557133D8CA62F61541B7187201D332684AE38E2465302564E323DD8CBA6441 |
| screenshots/d-save-fail/02-step-save-fail-modal-20261121-103337.png | 72,769 | D4623901BE7F9AAE9849422B8BB21CEBA0C614A5A88C666624153924A9D4528B |
| screenshots/d-save-fail/03-step-save-fail-notice-20261121-103337.png | 45,807 | 258B049C2840CC1D325B35D57614CB6E35B7BF77BDFB7F22231BAB5D94542F71 |
| screenshots/d-save-fail/walkthrough.log | 1,931 | 64DFFE9CDE697A53FB32B1FB2B303F004A9F07FE764DD3F36C2BD2F004673E56 |
| screenshots/d-save-fail/stderr.log | 561 | 66C69AE37092A9EE553AD090147A46AC7FB5DA4D9B0B5FE18C4019319ED9DF76 |
| screenshots/e-unsaved-close/01-w1350-p0-20261121-103410.png | 39,918 | 66CB6A6D484FA8CD9DDDE00BAE4D764FA5157D322E041D55B1CD440E13E3D338 |
| screenshots/e-unsaved-close/02-step-unsaved-close-modal-20261121-103410.png | 50,955 | 8E00958E77B17B397686C90BB2016A4B290F89D7C75D54DC8C8D23BA1542784C |
| screenshots/e-unsaved-close/walkthrough.log | 1,435 | 5F50E5048EB9876F25F836F7E58C9FD5BE36EB61D8966E37C2BC382D6BB20395 |

`stdout.log` 全部为 0 字节（SHA E3B0C442…B855），未列入。
