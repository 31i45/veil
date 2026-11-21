# after-v7 事实记录（最终产物 0d229eb）

产物 `target/release/veil.exe`：14,012,928 B，SHA-256
`8DF551F8D363C73DE1F35887042BEC7775E5AB8D1FFEDB50F365AE7E9B456763`，提交 `0d229eb`。
开始 20:15:36 校验（`exe-verify.txt` 的 START 行）；结束校验见同文件 END 行，两者一致。
DPI 144（150%）；物理屏 2560×1440；工作区 `0,0,2560,1368`；默认窗口外框 1372×1106 @593,127
（客户区 1350×1050 = 900×700 逻辑）。

相对 v6（c19a082）的产品改动仅为 `crates/veil-ui/src/i18n.rs` 文案：50 条失败/致命文案统一补
「下一步」或「重启应用」；链接、外部文件、配置未保存、Mermaid 快照失败均属本轮改动面。

## F1 十个场景

| # | 场景 | 结果 | 证据与哈希（相对 screenshots/） |
|---|---|---|---|
| 1 | 空文档首跑 | `未命名草稿 ｜ 已新建空白文档 ｜ 0 字 · 行 1，列 1`，无星号、无引导弹窗 | `a-empty-first-run/03-…20261121-201609.png`（25,457 B，`F6DBAD97…FB7125`） |
| 2 | 900px 窄窗 | 左 `未命名草稿`、中 `已新建空白文档`、右 `0 字`，无重叠 | `b-narrow-900/02-…201616.png`（23,986 B，`8788F491…F8B54`） |
| 3 | 700px 长名脏态 + 错误通知 | 浮层 6 行完整（无 `…`）；状态栏 `2026-11-21-ux-aft...stuvwxyz.md · 第四层 · 未保存 ｜ 28 字 · 行 4，列 26` | `f-700-longname-notice/02-…201639.png`（69,762 B，`0561CE22…96D6FD`） |
| 4 | 保存失败通知 | 模态：标题/文件名/原因/下一步/重试(R)/另存为/取消；Esc 后 2 行常驻通知含「下一步」 | `d-save-fail/02-…201621.png`（82,380 B，`C8FD48D5…AE3144A7`）、`03-…png`（48,482 B，`4E14FFA0…8341033C`） |
| 5 | 未保存关闭 | 三行逐按钮说明 + `保存(S)/不保存(C)/取消`，默认焦点保存 | `e-unsaved-close/02-…201632.png`（61,061 B，与 v6 同 SHA `104D4BBF…F84F33`） |
| 6 | 文件菜单 | 动作名/快捷键完整；三项禁用原因均为 `空文档没有可导出的内容` | `g-file-menu/02-…201649.png`（54,461 B，与 v6 同 SHA `EC05CC08…C8ECA165`） |
| 7 | 浅 / 深焦点环 | 两主题焦点环清晰（浅色蓝框、深色亮框） | `h-search-light/02-…201656.png`（99,824 B，同 v6）、`i-search-dark/02-…201703.png`（93,416 B，同 v6） |
| 8 | 崩溃恢复提示 | `已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 20:17。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）` | `k-recovery-run2/02-…201743.png`（49,754 B，`995A1B60…209A1BCD`） |
| 9 | 整窗 / 工作区 | 外框 1372×1106 @593,127；客户区 1350×1050；工作区 2560×1368；`inside_workarea=True` | `j-full-window/02-…201710.png`（95,617 B，同 v6 `C3A86FCB…DBACD1B`）、`window-rect.txt` |
| 10 | 最小尺寸 520×360 | 真实拖拽与 config 480×360 均钳到 520×360，通知全文可见；程序化 SetWindowPos 的 140 请求可绕过，通知被裁切（观察项，用户拖拽不可达） | `m-min-size/drag-min/{metrics.txt,notice.png}`、`logical-140x160/`、`physical-140x160/`、`restore-480x360/`、`restore-93x107/` |

第 10 项细节：

- `[A]` 真实鼠标右下角拖拽：客户区 780×540 物理 = **520×360 逻辑**，`clamped=True`，通知首行
  `保存失败：拒绝访问。 (os error 5)（写入` 与末行 `…Ctrl+Shift+S 另存为` 均可见。
- `[B]` 140×160 逻辑请求（SetWindowPos）：客户区 210×240 物理，未被钳制；通知仅显示原因前几行与
  `另存为` 尾词，`下一步` 主体被裁切。
- `[C]` 140×160 物理请求：客户区 140×160 物理（93.33×106.67 逻辑）；通知仅露出标题「保存失败」，
  正文与状态栏被裁切。
- `[D]` config `480×360` → 启动客户区 780×540 = **520×360**，`clamped=True`，界面完整。
- `[E]` config `93×107` 低于解析下限 → 整组忽略 → 默认 900×700。
- `[B][C]` 只能由外部 `SetWindowPos` 达成，用户拖拽与配置恢复均停在 520×360；故第 10 项按用户可达
  路径判 PASS，极端裁切作为观察项记录，不判 FAIL。

## F2 文案质检（本轮 34+ 失败 Key 的可截图代表项）

| Key / 路径 | 触发方式 | 实拍文案（逐字） | 证据 |
|---|---|---|---|
| `NoticeLinkUnsupported` | Ctrl+点击 `ftp://example.com` | `无法打开链接：不支持的链接地址 ftp://example.com；下一步：改用 http/https 链接，或复制到浏览器打开` | `l-link-unsupported/notice-t0.png`、`notice-t7s.png`，均 43,211 B / `B4E3919C…9E9A`，逐字节相同 |
| `NoticeLinkFailed` | Ctrl+点击 `mailto:%zz@example.com`（ShellExecuteW 返回 2） | `无法打开链接：ShellExecuteW 返回 2；下一步：检查系统默认浏览器，或复制链接手动打开` | `l-link-failed/notice-t{0,7s}.png`，均 37,645 B / `BE2822E3…8FA3`，逐字节相同 |
| `NoticeExternalDeleted` | 运行期删除被打开文档 | `文件已被外部删除；编辑器中内容仍在，下一步：可继续编辑，或用「另存为」保存到新位置` | `n-external-deleted/notice.png`（36,656 B，`B0B23716…C5653959`） |
| `NoticeExternalUnreadable` | 独占写+删共享句柄占用文档 | `文件暂不可读（可能被外部程序占用）；编辑器中内容仍在，下一步：可继续编辑或另存为` | `n-external-unreadable/notice.png`（36,705 B，`3C25749D…82FC787`） |
| `NoticeConfigNotSaved` | 只读 `config.toml`，设置里切换主题后确定 | `配置未保存：拒绝访问。 (os error 5)（写入 …\config.toml）；下一步：可检查配置目录的写入权限或磁盘空间后重试` | `q-config-not-saved/{notice.png,metrics.txt}`（110,087 B，`0C3F50E7…74D350`） |
| `NoticeMermaidSnapshotSaveFailed` | 预先把 `<doc>.assets` 建成普通文件，右键 mermaid 块 →「插入本地图片快照」 | `Mermaid 快照保存失败，未修改原文：当文件已存在时，无法创建该文件。 (os error 183)（创建 …\mermaid-snap.assets）；下一步：可检查目标目录是否可写后重新插入快照` | `q-mermaid-snap/{notice.png,menu.png,metrics.txt}`（52,557 B，`840FEC8F…6DF1CC`） |

六条均同时含「现状」与「下一步」。链接两条的 t0 与 t7s 逐字节相同（常驻）；外部文件两条为错误类通知，
按设计不超时，本轮各取一张实拍。
全量失败 Key 由 `all_failure_notices_offer_next_step` 单测穷举断言（`_work/i18n-test.log`，
`1 passed`），截图只做可截图的代表项。

## F3 与 v6 的像素差异

`_work/compare.txt`（v6 → v7 同场景逐张对比）：

- 逐字节相同：`e-unsaved-close/02`、`g-file-menu/02`、`h/i` 搜索、`j` 整窗，以及各场景首张
  `01-w*-p0` 空文档帧（`a/b/g/h/j` 首帧）。
- 差异集中在文案区域：`d-save-fail/02` 2,580 px、`03` 2,456 px（新增「下一步：」前缀）；
  `f-700/02` 4,398 px（通知与状态栏文案）；`k-recovery-run1/02` 4,369 px（状态栏字数/行列）；
  `k-recovery-run2` 417 px（恢复时间 20:17 变化）。
- 其余首帧差异均在内容区外（`content_region_px=0`），为插入光标/标题栏抗锯齿，无布局回归。

## F4 方法与限制

- 全部场景截图走 `tools/walkthrough.ps1`（PrintWindow，失败降级屏幕捕获）；每场景独立 `VEIL_HOME`。
- 最小尺寸、链接、外部文件、配置未保存、Mermaid 快照使用 `_work/` 内驱动脚本（复制自 `after-v6/_work`
  后改路径）；配置未保存与 Mermaid 快照为本轮新增探针，脚本与输入文档均在 `_work/`。
- 链接打开失败用白名单 `mailto:%zz@example.com`（ShellExecuteW 返回 2）；未改注册表、未劫持协议处理器。
- 只读文档在运行期 `attrib +R`、结束后恢复；未改仓库 tracked 文件，未 commit / push。
- 观察项（与 v6 相同）：外部 `SetWindowPos` 可绕过最小尺寸并在 140×160 裁切通知；用户拖拽与配置恢复均已精确钳制到 520×360 且界面完整。
