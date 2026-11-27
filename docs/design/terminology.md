# 界面术语与双语布局预算 v1

- 版本：v1（2026-11-27）
- 状态：生效，所有用户可见文案（菜单、右键菜单、浮层、弹窗、状态栏、提示与错误）的唯一用词依据
- 依据文件：`crates/veil-ui/src/i18n.rs`（`ZH_CN` 默认表 / `EN_US` 全量表）、`crates/veil-ui/src/tokens.rs`、`crates/veil-ui/src/view/settings.rs`、`crates/veil-ui/src/view/modal.rs`
- 对应关系：下表每行的 zh-CN / en-US 写法都必须能在 `i18n.rs` 中逐字找到（「现行词条」列给出 key）。新增概念先补本表，再改词条；不得出现本表有、代码无的写法。

## 一、术语表

同一对象只有一个名字；「禁用写法」列的词不得进入任何词条。

| # | 概念 | zh-CN 推荐 | en-US 推荐 | 禁用写法 | 现行词条（key = 文案） |
|---|---|---|---|---|---|
| 1 | 文件（磁盘上的实体） | 文件 | File | 档案 | `menu.file` = 文件 / File |
| 2 | 文档（编辑器打开的内容单元） | 文档 | document（句中）/ Document（独立标签） | 文稿、文本 | `error.export_draft_no_path` = 文档还没有保存位置，无法导出 / The document has no save location yet, so it cannot be exported |
| 3 | 保存 | 保存 | Save | 存储、存档 | `menu.file.save`、`button.save` = 保存 / Save |
| 4 | 另存为（动作名） | 另存为 | Save As | 保存为、存为、保存副本、另存为到… | `menu.file.save_as` = 另存为… / Save As… |
| 5 | 导出（写文件） | 导出 | Export | 汇出、输出 | `menu.file.export_html` = 导出 HTML… / Export HTML… |
| 6 | 关闭窗口 / 退出 | 关闭窗口＝关闭当前窗口；退出＝离开整个应用或某个模式 | Close Window / Quit（应用）、Exit（模式） | 用「退出」指关闭窗口；用「关闭」指退出应用 | `menu.file.quit` = 关闭窗口 / Close Window；`notice.fullscreen_off` = 已退出全屏 / Exited fullscreen。当前无应用级退出菜单项 |
| 7 | 查找 | 查找 | Find | 搜索（指查找栏时） | `menu.edit.find`、`search.placeholder` = 查找 / Find |
| 8 | 替换 | 替换 | Replace | 取代、替换成 | `menu.edit.replace`、`search.replace` = 替换 / Replace；`search.replace_placeholder` = 替换为 / Replace with |
| 9 | 大纲（标题导航面板） | 大纲 | Outline | 目录、概要、大纲面板 | `menu.view.outline`、`outline.title` = 大纲 / Outline |
| 10 | 目录（Table of Contents，PDF 目录页） | 目录 | Table of Contents | 大纲、索引 | `print.toc_title` = 目录 / Table of Contents；`settings.export_toc` = 导出 PDF 时包含目录页 / Include Table of Contents in PDF Export |
| 11 | 版本历史（功能） | 版本历史 | Version History | 历史记录、版本记录、修订历史 | `menu.file.version_history`、`history.title` = 版本历史 / Version History |
| 12 | 快照（版本历史中的存档条目） | 快照 | Snapshot | 历史文件、备份副本、快照文件 | `history.action.delete` = 删除快照 / Delete Snapshot |
| 13 | 草稿（未落盘的文档） | 草稿 | Draft | 草稿本、临时文档、未保存文档 | `common.untitled_draft` = 未命名草稿 / Untitled Draft |
| 14 | 只读 | 只读 | read-only | 只读模式、锁定（指文件权限时） | `open_failure.denied_next` = 请确认文件未被只读锁定或被安全软件拦截，取得权限后重试。 / Make sure the file is not read-only or blocked by security software, then try again once you have permission. |
| 15 | 远程图片（HTTP/HTTPS 链接的图片） | 远程图片 | remote images（句中）/ Remote Images（菜单与标签） | 网络图片、外链图片、在线图片 | `menu.view.allow_remote_images`、`settings.allow_remote_images` = 允许远程图片 / Allow Remote Images |
| 16 | 取消 | 取消 | Cancel（动作）/ Canceled（完成态） | 撤销（撤销是 Undo）；英式拼写 Cancelled | `button.cancel` = 取消 / Cancel；`notice.cancelled` = 已取消操作 / Canceled |
| 17 | 弹窗 / 对话框 | 应用内模态＝弹窗；操作系统的文件/打印对话框＝对话框 | dialog | 模态框、弹出框 | `notice.error_while_modal_open` = …请先关闭当前弹窗，再重试 / …close the current dialog, then try again；`modal.print_failed.body` = …重新打开打印对话框 / …reopening the print dialog |
| 18 | 撤销 / 重做 | 撤销 / 重做 | Undo / Redo | 回退、恢复（指 Undo 时） | `menu.edit.undo`、`menu.edit.redo` = 撤销 / Undo、重做 / Redo；恢复版本用 `history.action.restore` = 恢复此版本 / Restore This Version |

补充规则：

1. **全称与短名**：功能全称用于标题、菜单与成功提示（「版本历史」「版本快照」，如 `modal.delete_snapshot.title` = 删除版本快照 / Delete Version Snapshot、`modal.clear_history.title` = 清空版本历史 / Clear Version History）；行内按钮与动词短语在词干前省略「版本」（`history.action.delete` = 删除快照、`history.action.clear` = 清空历史）。短名只是省略，不是另一种叫法；不得写成「历史记录」「快照历史」等异名。
2. **「另存」的两种位置**：动作名（菜单项、按钮、对话框标题）一律「另存为」/ `Save As`；句子中作动词并带宾语时写「另存到…」「另存文档」（`notice.conflict_name_exhausted` = 或另存到其他位置；`notice.image_save_failed` = 或另存文档后重试），不得写「另存为到…」。
3. **汉字与英文混排**：英文专名（Veil、Markdown、Mermaid、HTML、PDF）在中文文案中保持原写法，不翻译、不加引号；快捷键写法固定为 `Ctrl+S`、`Ctrl+Shift+S` 形式，不用 `Cmd`。

## 二、省略号规则

1. **只有点击后会弹出对话框的菜单项 / 命令项才带省略号**：`menu.file.open`（打开… / Open…）、`menu.file.save_as`（另存为… / Save As…）、`menu.file.export_html`、`menu.file.export_pdf`、`menu.file.print`、`menu.edit.insert_image`、`ctx.insert_image`、`menu.view.settings`、`menu.help.export_diagnostics`。
2. **静默执行（立即生效，或无对话框的浮层）不带省略号**：`menu.file.quick_open`（快速打开 / Quick Open）、`menu.help.check_update`（检查更新 / Check for Updates）、`menu.view.outline`（大纲 / Outline，切换面板）以及右键菜单里直接改写文档的项。
3. **省略号字符固定为 `…`（U+2026），前置不加空格**：写 `Save As…`，不写 `Save As ...`、`Save As...`、`Save As …`。
4. **弹窗按钮不加省略号**：按钮本身就是确认动作，且需要承载助记符后缀（见第三节），因此 `button.save_as` = 另存为 / Save As，不写「另存为…」。
5. **状态栏提示、成功/失败文案不以省略号结尾**；`i18n::clip_error` 对超长错误的自动截断属运行时行为，不属于文案写法。

## 三、助记符规则

1. 助记符**不写进词条**：`ModalButton` 用独立字段 `mnemonic` 保存（`crates/veil-ui/src/view/modal.rs`），上屏由 `display_label()` 追加「半角空格 + 半角大写括号」，例如「保存 (S)」；英文界面为 `Save (S)`。
2. 词条表里只放动作名：`button.save` = 保存 / Save。禁止写成「保存(&S)」「保存(S)」「保存（S）」（全角括号）；绘制层不解析 `&`，会把 `&S` 直接画出来。
3. 中文动作名不含拉丁字母，因此助记符必须由调用点显式指定（`.with_mnemonic('s')`）；未指定时该按钮不显示括号后缀（如 `button.cancel`）。
4. 助记符取动作对应的英文键且同一弹窗内不重复：保存 S、不保存 C（取 Cancel 语义，避让 S）、重试 R、删除 D、保留当前 K、另存为 A、重新加载 R、打开其他文件 O。
5. 助记符只影响「弹窗按钮」；菜单栏与右键菜单不显示括号助记符，其快捷键提示由 `shortcut.*` 词条单独给出。

## 四、英文写法规则

1. **菜单项、右键菜单项、按钮、设置标签、对话框标题用 Title Case**：实词首字母大写；冠词与短介词（a / an / the / in / on / of / to / for / with / or / and）小写，但首词与末词一律大写。示例：`Save As…`、`Insert Row Above`、`Include Table of Contents in PDF Export`。
2. **`as` 在 Save As 中大写**：按 en-US 编辑器惯例写 `Save As…`，不写 `Save as…`。
3. **句末标点**：短语型菜单项、按钮、标签、标题不加句号；完整句子型提示与错误正文以句号结尾。
4. **大小写分场景**：状态栏片段用句内大小写（` · Unsaved`、`Saved: {}`、`Typewriter mode: {}`）；独立标签用 Title Case（`Typewriter Mode`、`Version History`）。
5. **拼写用 en-US**：取消的完成态统一 `canceled` / `Canceled`，禁用英式 `cancelled` / `Cancelled`（`notice.cancelled` = Canceled、`notice.export_cancelled` = Export canceled、`notice.save_as_cancelled` = Save As canceled）。
6. **专名不翻译**：Veil、Markdown、Mermaid、HTML、PDF、ProgID、WIC、WinHTTP、UTF-8 保持原写法；快捷键写 `Ctrl+Shift+S`。
7. **占位符**：固定写 `{}`，不写 `{0}`、`{name}`；中英文词条的 `{}` 数量与首尾位置必须一致（由 `en_us_entries_keep_placeholder_parity` 与 `every_key_has_en_us_entry_with_placeholder_parity` 两个用例锁定）。

## 五、双语布局预算

**基准**：UI 字号 `typography::UI_SIZE = 13`（`crates/veil-ui/src/tokens.rs`）；预算按 1.0× 逻辑像素定义，缩放时字号与列宽同比放大，字符预算不变。

**字宽估算系数**：英文按 **0.62 em/字符**（13px 下 ≈ 8.06px/字符）。
换算依据：`view/settings.rs` 实测最长英文标签「Include Table of Contents in PDF Export」（39 字符）约占 312px，折合 8.0px/字符 ≈ 0.615em；向上取 0.62em 作保守上界，高分屏回退到更宽字体时仍不越界。

**弹窗可用宽度**（`surface::DIALOG_MAX_WIDTH = 760`）：

```
内宽        = 760 − 2 × DIALOG_PADDING(20) = 720px
标签列上限  = min(内宽 × 50%, 320px) = 320px      // view/settings.rs::layout 的 label_col_w_max
控件列      = 380px，列间距 = 10px                // 320 + 10 + 380 = 710 ≤ 720
```

**字符上限**：

| 位置 | 可用宽度 | 字符上限 | 说明 |
|---|---|---|---|
| 弹窗标签列（设置项标签） | 320px | **39** 字符 | `floor(320 / 8.06) = 39`；当前最长 39（「Include Table of Contents in PDF Export」「Custom Body Margin (4–200, Custom Only)」） |
| 弹窗内单行控件文案（菜单 / 右键 / 按钮 / 历史动作 / 查找 / 大纲 / 开关 / 设置） | 720px | **89** 字符 | `floor(720 / 8.06) = 89` |
| 提示与错误正文（`notice.*`、`modal.*.body`） | — | 不设字符上限 | 由绘制层换行，禁止放进标签列 |

规则：

1. 新增或修改英文标签时，先按上表估算宽度；超限就改短，不靠缩字号或压列宽解决。
2. 标签列上限 320px、控件列 380px、`UI_SIZE = 13` 三者任一变化，必须同步更新本节数值与门禁测试。
3. 门禁：`crates/veil-ui/src/i18n.rs::tests::longest_english_strings_fit_dialog_budget`（系数与预算写在测试内，失败信息给出越界词条、字符数与 key）。

## 六、维护与门禁

1. 新增文案：先查本表；表内没有的新概念先补一行（含禁用写法），再在 `i18n.rs` 的 `keys!` 宏加 zh-CN，并在 `EN_US` 表补齐英文。
2. 两张语言表必须全量且 key 集合一致，占位符数量一致——由 `every_key_has_zh_cn_entry`、`every_key_has_en_us_entry_with_placeholder_parity`、`en_us_entries_keep_placeholder_parity` 锁定。
3. 本表与 `i18n.rs` 的对应关系由人工核对：修改本表任何「推荐写法」时，必须在同一次提交内改掉所有引用该文案的词条与测试断言。
4. 验收命令：`cargo test -p veil-ui i18n::` 与 `cargo test --workspace`。
