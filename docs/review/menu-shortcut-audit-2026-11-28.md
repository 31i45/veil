# 菜单 / 快捷键 / 命令面板一致性审计（2026-11-28）

范围：`crates/veil-ui` 的主菜单、键盘翻译、命令面板与右键菜单。只核对提示与绑定是否一致，
不改变任何既有键位。

事实来源（代码为准，非文档转述）：

- 菜单表：`crates/veil-ui/src/chrome/mod.rs` 的 `MENUS: [MenuDef; 4]`
- 键盘翻译：`crates/veil-ui/src/input.rs` 的 `translate(key, mods)`
- 分发：`crates/veil-ui/src/interaction_menu.rs` 的 `perform_menu`；`interaction.rs` 的 `perform_action`
- 命令面板：`crates/veil-ui/src/view/quick_open.rs` 的 `command_items()`（展开 `MENUS` 非分隔线项）
- 右键菜单：`crates/veil-ui/src/view/ctx_menu.rs` 的 `items_for(kind)` 与 `main_equivalent`

## 本轮修正

| 项 | 修正前 | 修正后 |
|---|---|---|
| 复制代码块入口 | 仅右键菜单（`CtxKind::CodeBlock`），键盘 `Ctrl+Shift+C` 可用但无菜单入口 | 编辑菜单新增「复制代码块」，快捷键提示 `Ctrl+Shift+C`；命令面板自动收录 |
| 可用性 | 菜单无该项，无法置灰解释 | 光标不在围栏代码块内时置灰，右侧显示原因「光标不在代码块内」 |
| 词条 | 仅有 `ShortcutCopyCode` | 新增 `MenuEditCopyCode`（中「复制代码块」/英「Copy Code Block」）与 `DisabledNoCodeBlock` |

启用条件（单一判据）：`App::menu_flags().can_copy_code` = `code_block_at_caret().is_some()`，
即光标所在块为 `BlockKind::FencedCode(_)`（含 Mermaid 围栏）。菜单、命令面板、`Ctrl+Shift+C`、
右键复制按钮共用同一块判定；不一致时不会出现「可点却没有可复制内容」。

## 审计矩阵

「命令面板」列：✓ = `MENUS` 展开后必然收录（面板不区分可用性）。
快捷键列均已在 `translate` / `route` 中逐条核对可达；无提示的动作明确不编造快捷键。

| 用户可执行能力 | 菜单入口 | 快捷键提示 | 命令面板 | 备注 |
|---|---|---|---|---|
| 新建 | 文件 → 新建 | Ctrl+N | ✓ | |
| 新建窗口 | 文件 → 新建窗口 | Ctrl+Shift+N | ✓ | `mode::route` 对 Ctrl+Shift+N 有跨模式放行 |
| 打开… | 文件 → 打开… | Ctrl+O | ✓ | 对话框 |
| 快速打开 | 文件 → 快速打开 | Ctrl+P | ✓ | |
| 保存 | 文件 → 保存 | Ctrl+S | ✓ | 无改动/未命名时置灰并解释 |
| 另存为… | 文件 → 另存为… | Ctrl+Shift+S | ✓ | 对话框 |
| 导出 HTML… | 文件 → 导出 HTML… | Ctrl+E | ✓ | 空文档置灰 |
| 导出 PDF… | 文件 → 导出 PDF… | Ctrl+Shift+E | ✓ | 空文档置灰 |
| 打印… | 文件 → 打印… | Ctrl+Alt+P | ✓ | AltGr 例外在 `translate` 单独放行；Alt 布局已知取舍 |
| 版本历史 | 文件 → 版本历史 | Ctrl+Shift+H | ✓ | |
| 最近文件 | 文件 → （动态插入） | 无 | 否 | 动态项，不在命令面板 |
| 关闭窗口 | 文件 → 关闭窗口 | 无 | ✓ | 无提示，不编造 |
| 撤销 | 编辑 → 撤销 | Ctrl+Z | ✓ | 无撤销步置灰 |
| 重做 | 编辑 → 重做 | Ctrl+Shift+Z | ✓ | `Ctrl+Y` 也可达，但提示只显示一种（记录项） |
| 剪切 | 编辑 → 剪切 | Ctrl+X | ✓ | |
| 复制 | 编辑 → 复制 | Ctrl+C | ✓ | |
| 粘贴 | 编辑 → 粘贴 | Ctrl+V | ✓ | |
| 全选 | 编辑 → 全选 | Ctrl+A | ✓ | |
| 加粗 | 编辑 → 加粗 | Ctrl+B | ✓ | |
| 斜体 | 编辑 → 斜体 | Ctrl+I | ✓ | |
| 链接 | 编辑 → 链接 | Ctrl+K | ✓ | |
| 复制代码块 | 编辑 → 复制代码块 | Ctrl+Shift+C | ✓ | 本轮新增；光标不在围栏块内置灰并解释 |
| 查找 | 编辑 → 查找 | Ctrl+F | ✓ | |
| 替换 | 编辑 → 替换 | Ctrl+H | ✓ | |
| 插入图片… | 编辑 → 插入图片… | Ctrl+Shift+I | ✓ | |
| 大纲 | 视图 → 大纲 | Ctrl+Shift+O | ✓ | |
| 放大 | 视图 → 放大 | Ctrl+= | ✓ | `Ctrl+Shift+=` 同样放大（translate 不区分 Shift） |
| 缩小 | 视图 → 缩小 | Ctrl+- | ✓ | |
| 还原缩放 | 视图 → 还原缩放 | Ctrl+0 | ✓ | |
| 全屏 | 视图 → 全屏 | F11 | ✓ | 无需修饰键 |
| 跟随系统 / 浅色 / 深色 | 视图 → 跟随系统/浅色/深色 | 无 | ✓ | 当前项显示「当前」 |
| 源码模式 | 视图 → 源码模式 | Ctrl+/ | ✓ | |
| 专注模式 | 视图 → 专注模式 | 无 | ✓ | 开关项显示「当前」 |
| 打字机模式 | 视图 → 打字机模式 | 无 | ✓ | 开关项显示「当前」 |
| 允许远程图片 | 视图 → 允许远程图片 | 无 | ✓ | 勾选 = 允许 |
| 设置… | 视图 → 设置… | Ctrl+, | ✓ | 对话框 |
| 关于 Veil | 帮助 → 关于 Veil | 无 | ✓ | |
| 导出诊断信息… | 帮助 → 导出诊断信息… | 无 | ✓ | 对话框 |
| 检查更新 | 帮助 → 检查更新 | 无 | ✓ | 只开系统浏览器 |

统计：静态非分隔线动作 40 项；显示快捷键 30 项；命令面板候选 = 40（不含动态最近文件）。

## 右键专属白名单（仅右键，无主菜单/命令面板入口）

共享动作（不列入白名单）：剪切、复制、粘贴、全选、加粗、斜体、链接、插入图片。

仅右键可执行的能力（`CtxAction` → 无 `main_equivalent`）：

- 链接：在浏览器中打开（`OpenLink`）、复制链接地址（`CopyLink`）
- 表格结构：上方插入行、下方插入行、删除行、左侧插入列、右侧插入列、删除列
- 表格对齐：左对齐、居中、右对齐
- Mermaid：在 mermaid.live 预览、应用内渲染预览、插入渲染图片、复制源码、插入流程图/时序图/类图模板
- Details：折叠、展开
- 图片：复制源路径、重新加载、允许远程图片（全局）
- 不可执行提示项（占位说明，不参与命中）：图片缺失提示

说明：`CopyCodeBlock` 修正前属于本白名单（右键专属），修正后升级为「编辑菜单 + `Ctrl+Shift+C` + 命令面板」
三入口；右键仍保留，文案与提示同源。

## 非主流键位（只记录，不修改）

| 键位 | 绑定 | 说明 |
|---|---|---|
| Ctrl+Alt+P | 打印 | 为给命令面板让出 Ctrl+Shift+P；与 AltGr（Ctrl+Alt）字符输入冲突，`translate` 对 P 单独例外 |
| Ctrl+Shift+C | 复制代码块 | 常见编辑器多用于复制/终止进程；此处语义为「复制光标所在围栏块」 |
| Ctrl+Shift+H | 版本历史 | 与部分编辑器「格式化/替换」习惯不同 |
| Ctrl+Shift+O | 大纲 | VS Code 中为「转到符号」 |
| Ctrl+Shift+E | 导出 PDF | VS Code 中为资源管理器 |
| Ctrl+/ | 源码模式 | 常见编辑器为注释切换 |
| Ctrl+, | 设置 | macOS 习惯，Windows 应用较少使用 |
| Ctrl+K | 链接 | 与 VS Code 的组合键前缀（chord）习惯不同 |
| Ctrl+= | 放大 | 也可 Ctrl+Shift+=；提示只显示前者 |

## 已知差异与待观察项（本轮不修改键位）

1. **命令面板不复用菜单的禁用态**：`quick_accept` 直接 `perform_menu`，不检查 `action_enabled`。
   复制代码块从面板执行且光标不在代码块内时，会走到 `copy_code_block_at_caret` 并给出
   「光标不在代码块内」错误通知（可见、不静默），但面板本身不置灰。主菜单与右键均置灰并解释。
2. **重做的备用键 `Ctrl+Y`**：`translate` 支持，但菜单/面板只提示 Ctrl+Shift+Z。属提示不全，
   不是不可达；如需完整披露需改提示，本轮按「只补漏、不改键位」保留。
3. **Ctrl+Shift+C 与查找栏**：查找栏聚焦时 `passes_through_search` 不含 `CopyCodeBlock`，
   该组合被查找栏模式吞掉（不复制、不粘贴）；需先退出查找栏。提示面向文档模式，未额外标注。
4. **动态最近文件**：菜单有入口、无快捷键、不进命令面板；`menu_command_matrix` 已断言其提示为空。

## 验证

```
cargo test -p veil-ui --release --lib chrome -- --test-threads=1
cargo test -p veil-ui --release --lib quick_open -- --test-threads=1
cargo clippy -p veil-ui --all-targets -- -D warnings
```

结果（2026-11-28）：chrome 过滤 115 passed；quick_open 过滤 21 passed；
`cargo clippy -p veil-ui --all-targets -- -D warnings` 通过；
新增 i18n 词条另跑 `cargo test -p veil-ui --release --lib i18n` 26 passed。
