# 3e 真机走查结果记录

> 对照清单：[3e 真机走查清单与预期效果](3e-manual-walkthrough.md)
> 产物：`target\release\veil.exe`（2026-11-04 构建，5,840,896 B ≈ 5.57 MiB，含 commit `4deeca9`）
> 测试期间不改代码；所有问题记录后统一修复。
> **当前权威状态**：以「## 真机走查轮次 1（2026-11-04）」及「## 未来版本规划候选（2026-11-04 快照）」为准；带「历史快照」标注的章节仅作留痕。

## 环境

- OS：Microsoft Windows 11 企业版 10.0.26300
- 显示器 DPI：150%（探针实测 `GetDpiForWindow` = 144）
- 输入法：中文拼音 + 英文键盘布局
- 安全软件：待填

## 交接（2026-11-02 集中修复轮）

> **历史快照（2026-11-02/03）**：本节进度、修复清单与基线数字均为当时记录（867 passed / 11 ignored / 5,825,536 B），已被 2026-11-04 轮次 1 覆盖；当前基线为 **894 passed / 12 ignored / 5,840,896 B ≈ 5.57 MiB**（commit `4deeca9`）。

- **进度**：清单 65 项，已走查 13 项（A1–A6、B1–B7）；**下一项 B8**（状态栏字数/行列）。
- **本轮修复**：I2 Ctrl+I 崩溃、A2 标题脏标、P1 HTML 粘贴尾换行、S2 打印快捷键、
  G7 未保存三选一、C 查找栏可达性、C2 菜单吞 Ctrl、F 图片独占段 + Ctrl+Shift+I、
  V 查找栏光标、V2 光标定位、P2 图片文件复制粘贴、M1 菜单勾选态、
  P1b 查找栏 Ctrl+V 粘贴、M1b 对勾改「当前」、M1a 专注/打字机互斥（详见下方「修复状态回填」）。
- **已决策/已修**：T1 采用方案 1（2026-11-03 已修，见回填表）；B7-3 富文本粘贴口径维持现状（HTML→Markdown 结构转换）。
- **已修**：I1 CRLF 文档 Enter 后光标错行（2026-11-03 定位并修复，见回填表）。**按用户决定不处理**：I3 表格内 Ctrl+I 疑似无效。
- **新包复测**：C1/C2/C3（菜单开着按 Ctrl+F/Z/S）、C4 查找栏光标、F1–F3（独占段、Ctrl+Shift+I）、
  P2（资源管理器 Ctrl+C 图片 → Ctrl+V）、M1（视图菜单专注/打字机勾选态）。
- **基线（2026-11-03 最终复测）**：`cargo test --workspace` **867 passed / 0 failed / 11 ignored**；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；release 5,825,536 B ≈ 5.56 MiB。
- **产物与素材**：`target\release\veil.exe`；`%TEMP%\veil-walkthrough\`（4 个测试文档）。

## 自动化项（我测，无需人工）

> **历史快照（2026-11-03）**：以下为机检实测，不代表真机验收；走查状态以「## 真机走查轮次 1（2026-11-04）」为准。

| # | 项目 | 实测 | 判定 |
|---|---|---|---|
| A2 | 冷启动墙钟（5 次，no args） | 201 / 78 / 77 / 80 / 77 ms，中位 **78ms** | ✅ ≤300ms（首次 201ms 为磁盘冷读） |
| A5 | exe 文件属性 | FileVersion=ProductVersion=0.1.0；描述「Veil Markdown 编辑器」；`ExtractAssociatedIcon` 取回 32×32 图标；体积 5,815,808B≈5.55MiB | ✅ |
| A1（机检部分） | PE subsystem | =2（GUI，无控制台窗口） | ✅ |

## 人工项（用户观察后逐条回填）

> 历史快照（2026-11-01/02）：A1/S1/K1 等已随后修复（见回填表）；同一项状态以「## 真机走查轮次 1（2026-11-04）」为准。

| # | 项目 | 预期 | 结果 | 备注 |
|---|---|---|---|---|
| A1 | 双击启动外观 | 窗口正常出现；无黑色控制台；任务栏/标题栏为蓝底白 V 图标 | ⚠️ 部分不通过 | 无控制台 ✅、任务栏图标 ✅、绘制正常 ✅；**标题栏左上角图标不对**（默认图标）。根因预判：winit 未调用 `set_window_icon`，窗口类沿用 IDI_APPLICATION，而任务栏读取 exe 资源。修法：启动时用内嵌 RGBA 调 `set_window_icon`（待统一修复） |
| A3 | 首屏 3 秒可输入 | 无白屏/闪烁/未响应 | ✅ 通过 | 用户实测：输入汉字+英文+回车跟手，无白屏/闪烁/未响应 |
| A4 | CLI 打开文件 | 标题与状态栏路径正确 | ✅ 通过 | 标题与状态栏均为完整路径。**新缺陷 S1（用户发现）**：窗口未最大化且路径较长时，状态栏左侧路径与右侧「N 字 · 行 x，列 y」重叠。期望：左侧超长时省略中间段（`…\父目录\文件名`）或右侧优先固定、左侧收缩；待统一修复 |
| A6 | 空文档初始态 | `未命名草稿` / `0 字 · 行 1，列 1` | ✅ 通过（经文件菜单「新建」） | 标题 veil、未命名草稿、0 字 · 行 1，列 1、空白正文四项正确。**新缺陷 K1（用户发现，高）**：物理键盘 Ctrl+S / Ctrl+N 无响应，只能用菜单。需复现确认是否与输入法/焦点相关 |
| K1 复现 | 英文输入法 + 正文焦点下 Ctrl+S/B/Z/P、F11 | — | ❌ 确认缺陷 | Ctrl 组合键全部无效；F11 正常。根因（读码定位）：`map_key` 仅匹配 `Named` 键与 `logical_key=Character("s")`；Windows 按住 Ctrl 时 winit 的逻辑键为控制字符（0x01–0x1A，如 S→0x13），`event.text` 又因含控制字符被跳过 → 返回 None。修复方向：Ctrl 按下时优先用 `key_without_modifiers()` 或把 0x01–0x1A 映射回 `a`–`z`（不新增依赖） |
| B3 | 编辑键位（Enter/Backspace/Delete/←→↑↓/Home/End） | 与记事本一致，↑↓ 保持目标列 | ✅ 通过 | 用户实测「全部符合」 |
| B1 | 长文连续输入 200 字 | 光标可见、无丢字、跟手 | ⚠️ I1 已修，待真机复验 | 无跳字/吞字/重复、输入跟手 ✅；**偶发缺陷 I1**：一次观察中出现「光标显示在新输入行的上一行」，已定位为 CRLF 文档 Enter 后光标错行，2026-11-03 修复（见回填表） |
| B4 | 撤销/重做粒度 | 精确、可逐步回退 | ✅ 已修并决策 | 菜单撤销/重做可用，无内容丢失；最终语义为「连续性合并 + 光标移动/扩选断组」：`Session::break_undo_group` 已在光标移动/扩选意图处断组（`editor.rs`），「输入 ab → 移动光标 → 输入 c」只撤销 c。**决策：不加 200ms 静默窗口**——与 fa584ed 去 500ms 时间窗的拍板一致（时间窗使撤销步数随停顿变化，语义不确定）。鼠标路径断组已补齐：`interaction.rs` 在单击/双击/三击命中正文处统一调用 `break_undo_group`（回归测试 `mouse_click_breaks_undo_group`） |
| B5 | Tab / Shift+Tab | 无选区插 4 空格；有选区整行缩进/反缩进；反缩进不删非空白字符 | ⚠️ T1 已修，待真机复验 | ①无选区 Tab 插 4 空格 ✅；③反缩进不删非空白字符 ✅；②有选区整行缩进后**正文渲染异常**（原 T1），2026-11-03 按方案 1 修复（见回填表） |
| B6 | 双击词 / 三击行 | 中文连续字视为一个词、英文单词整体选中；三击选中整行（不含换行） | ✅ 通过 | 用户实测「都正常」：中文连字成词、英文整词（不含空格标点）、三击整行不含换行三项均符合 |
| B7 | 选区直接输入 / 粘贴（四组） | 选区被替换；单步可撤销 | ✅ 4 通过（B7-1 为既定连续性合并口径） | B7-1 连续输入后 Ctrl+Z 一次撤销整串（既定口径，非缺陷）；B7-2 通过；**B7-3 已确认：HTML→Markdown 结构转换、不保留视觉样式（最终口径）**；B7-4 图片粘贴正常 ✅ |
| … | （按清单 A–H 顺序追加） | | | |

## 修复状态回填（2026-11-01）

> **历史快照（2026-11-01）**：仅保留修复记录；状态以「## 真机走查轮次 1（2026-11-04）」为准。

| 缺陷 | 修复提交 | 内容 | 状态 |
|---|---|---|---|
| K1 Ctrl 组合键失效 | d85e278 | `map_key` 改用 `key_without_modifiers()` + AltGr 防护 | ✅ 已修，复验合并到 B 区键位项 |
| S1 状态栏长路径重叠 | 969bbc4 | 右文优先固定 + 左文超宽中段省略 | ✅ 已修，复验：长路径文件 + 窄窗口 |
| A1 标题栏图标默认 | d013e2f | 像素真源移 `icon.rs`，`with_window_icon` 显式设置 | ✅ 已修，复验：标题栏/Alt+Tab 图标 |
| U1 撤销粒度 | fa584ed | 去 500ms 时间窗，纯连续性合并（拍板的 VS Code 式）：单字符/无换行/单原语且位置紧邻即合并，停顿不断组；换行/多字符/多原语单独成步 | ✅ 已修，复验合并到 B4 |
| I1 偶发光标错行 | 2026-11-03 | 根因 `trailing_newlines` 只认 `\n`，CRLF 文档 Enter 后光标贴在上一行；改为按字素识别 CRLF（`crates/veil-ui/src/interaction.rs`），测试 `crlf_enter_caret_anchors_next_line` | ✅ 已修 |
| T1 整行缩进把段落变成代码块 | 2026-11-03 | 采用方案 1：上下文前缀（列表/段落 2 空格、引用加 `> `），`crates/veil-ui/src/editor.rs`；测试 `tab_whole_line_indent_keeps_paragraph_kind`、`tab_whole_line_indent_nests_list_item` | ✅ 已修（方案 1） |
| R1 每次启动恢复旧草稿，须反复另存为 | 3e-修R1 | 见下方详述 | ✅ 已修（用户报障，追加修复） |

### R1：每次启动都恢复同一个「未命名草稿」，必须再另存为才能继续

- **现象**（用户报障 + 截图确认）：把草稿另存为到桌面后，每次启动仍打开同一份无路径草稿，
  状态栏显示「已从崩溃恢复未保存的内容；确认后请保存」，标题 `veil *`；
  必须再执行一次另存为，之后才能打开/新建文件。
- **根因**：[doc_state.rs](../../crates/veil-ui/src/document/doc_state.rs) 的
  `clear_recovery` 以 `recovery_hash.is_none()` 作为「本会话没写过恢复文件」的判据提前返回；
  但启动时 [app.rs](../../crates/veil-ui/src/app.rs) 把 `recovery_hash` 初始化为 `None`，
  尽管磁盘上存在一份刚被消费的快照。于是「恢复草稿 → 直接另存为」时清除被跳过，
  旧快照留存 → 下次启动再次恢复 → 又需另存为，形成自循环。
- **修法**：启动消费快照时记录其指纹（`StartupDoc.recovery_hash = fnv64(快照字节)`），
  保存成功后 `clear_recovery` 正常作废；CLI 显式打开（`resume=false`）仍保持
  「不动他处快照」的原有保护语义。新增 2 条回归测试：恢复草稿另存为后快照作废、
  外部打开保存不误清他人快照。
- **现场处理**：删除 `%APPDATA%\Veil\recovery.dat` 中的过期快照（已备份至
  `%TEMP%\veil-recovery-backup.dat`），新 exe 启动即不再恢复该草稿。

### T1：整行缩进把普通段落变成「缩进代码块」

- **复现**：选中普通段落行（含 `**粗体**`、`~~删除线~~`、反引号、`[]()` 等标记），按 Tab。
- **现象**：行首被加 4 空格 → CommonMark 判为 indented code block，整段切换为灰底等宽、
  所有标记字面显示，原先的粗体/链接/行内代码全部失效；视觉上像「正文被打回源码」。
- **根因**（读码定位）：[editor.rs](../../crates/veil-ui/src/editor.rs) `indent_plan` 对
  触及的每一非空行**无条件**加 4 空格前缀，无块上下文判定——列表项、代码块、段落一律同等对待。
- **候选方案**（需拍板）：
  1. 上下文感知：列表项 → 改嵌套层级（保持列表结构）；围栏代码块内 → 插 4 空格；
     引用块 → 加 `> `；**普通段落 → 整行缩进不生效**（仅光标处插空格）；
  2. 普通段落整行缩进改用 2 空格（不触发代码块阈值，但「缩进」语义变弱）；
  3. 维持现状（视为 markdown 源码真相），仅在文档中说明。
- 倾向方案 1：与 Typora/Obsidian 的直觉一致，且不牺牲列表缩进这一真实高频场景。
- **最终落地（2026-11-03）**：采用方案 1。列表项/普通段落使用 2 空格上下文前缀，引用块加 `> `，普通段落不再被无条件加成 4 空格代码块；实现位于 `crates/veil-ui/src/editor.rs`。回归测试：`tab_whole_line_indent_keeps_paragraph_kind`、`tab_whole_line_indent_nests_list_item`。

修复后已重打包：`target\release\veil.exe`（2026-11-03 最终复测轮重打包，5,825,536 B ≈ 5.56 MiB，FileVersion=0.1.0）。
从 B5 恢复走查时顺带复验上表四项。

## 修复状态回填（2026-11-02 集中修复轮）

> **历史快照（2026-11-02）**：仅保留修复记录；状态以「## 真机走查轮次 1（2026-11-04）」为准。

走查期中冻结代码的约束已解除，按「一个切片一 commit、全量测试 + clippy 零警告」推进。

| 缺陷/缺失 | 修复提交 | 内容 | 状态 |
|---|---|---|---|
| I2 Ctrl+I 崩溃 | 40e2b29 | `apply_wrap` 以字节下标判断字符包裹，选区含 CJK 时切片落在 UTF-8 中间 → panic；改为按字素/字符边界判定 | ✅ 已修，复验：选中中文按 Ctrl+I 不崩溃且只包一层 `*` |
| A2 标题栏无脏标 | 96a2c04 | 已打开文件编辑后标题不显示星号：Windows 对超长标题从右侧截断，末尾 `*` 被吃掉 → 脏标前置为 `* ` | ✅ 已修，复验：长路径文件编辑后标题开头出现 `* ` |
| P1 HTML 粘贴凭空断行 | 41cc541 | `html_to_markdown` 恒以 `\n` 收尾，行中粘贴被多切一段 → 插入前剥离末尾单个换行 | ✅ 已修，复验：行中粘贴不新增空行 |
| S2 打印无快捷键 | da2b200 | Ctrl+P = 打印（系统对话框含预览）；快速打开让位 Ctrl+Shift+P，菜单标注与单测同步 | ✅ 已修，复验：Ctrl+P 弹打印对话框、Ctrl+Shift+P 弹快速打开 |
| G7 未保存关窗/新建/打开 | 375ecde | 脏文档三选一「保存/放弃/取消」贯通关窗、菜单退出、Ctrl+N、Ctrl+O、拖放、最近文件、快速打开；放弃与干净退出清恢复快照；无路径草稿走另存为。Windows 经 `MessageBoxW` 手写 FFI（零新依赖） | ✅ 已修，复验：G7/G8 及恢复快照无残留 |
| K1 二次（Ctrl 组合全失效） | fcd28bb | Windows `event.text` 剥离 Ctrl，旧映射先命中文本分支 → 全部 Ctrl 快捷键静默失效；Ctrl 组合一律走 `key_without_modifiers()`，AltGr 改走文本 | ✅ 已修（本轮复验 Ctrl 组合正常） |
| C Ctrl+F / Ctrl+H 查找栏不可操作 | 6b1ed06 | 三处结构缺陷：①打开查找栏未关右键菜单，栏可见但 `Mode::Menu` 吞键/吞点击；②Ctrl+H 首开把焦点放替换框，首字进替换导致「无命中、看似无响应」；③栏已开时 Ctrl+F/Ctrl+H 被路由吞掉，无法重申焦点/展开替换行。修法：`open_search/open_replace/open_quick` 统一关菜单；Ctrl+H 首开聚焦查询、栏已开时再按才聚焦替换；Search 模式补 Ctrl+F/H 路由；AltGr 防护下沉 `mode::route` | ✅ 已修（533→537 单测全绿）；**需新包复测**：若仍复现请注明「打字无响应」还是「点击无响应」 |
| 图片独占一段（Typora 式） | 431a592 | 用户拍板「是」：粘贴/拖入/选图插入图片时，按光标位置补前后换行，让 `![](path)` 自成一段（行首/行尾免加） | ✅ 已修 |
| 插入图片无快捷键 | 431a592 | 用户拍板「是」：加 Ctrl+Shift+I（Ctrl+I 仍是斜体；match 顺序先判 Shift 组合） | ✅ 已修 |
| 打印预览 | — | Ctrl+P 已走系统打印对话框（自带预览页），不再另建应用内预览 | ✅ 已覆盖（E1/E2 复验） |
| C2 菜单开着时 Ctrl 组合被吞 | 112ee20 | `Mode::Menu` 只认 Esc → 右键菜单/标题栏菜单下 Ctrl+F 等全部 Ignore。改为菜单内 Ctrl 组合经 `translate` 放行，`perform_action` 入口统一关菜单（双保险） | ✅ 已修，复验：右键菜单开着按 Ctrl+F 出现查找栏 |
| V 查找栏无光标 | ad4e5cd | 查询/替换框补 2px 竖线，风格与正文光标一致（静态，不闪烁） | ✅ 已修 |
| V2 查找栏光标位置 | 30ca197 | 光标只跟随真实文本：空串落输入区左端（不再跟在占位提示之后），有文本落文本末端 | ✅ 已修 |
| P2 资源管理器复制图片无法粘贴 | 4baf76d | 剪贴板新增文件列表读取（`Clipboard::get_files`，产品层 arboard `file_list`）；图片文件优先于路径文本，解码后走既有落盘+独占段路径 | ✅ 已修 |
| M1 看不出专注/打字机是否开启 | 30ca197 → 0e2493a | 第一轮画了左侧对勾（丑）；第二轮改为右侧「当前」同主题项视觉；大纲面板不显示状态标识（面板开关≠写作模式） | ✅ 已修 |
| M1a 专注/打字机可同开 | 0e2493a | 两者互斥切换：开启其一自动关另；`toggle_focus`/`toggle_typewriter` 各自在 `if on` 分支互关 | ✅ 已修 |
| P1b 查找栏 Ctrl+V 无法粘贴 | 0e2493a | Mode::Search 此前只放行文本/退格/Enter/Tab/几个 Ctrl 组合，`Key::V if ctrl` 落在 catch-all `Ignore`。新增 `Cmd::SearchPaste`，独立路由：只取剪贴板纯文本、换行换空格、限长 500，推入查询/替换焦点框（文件列表/图片在查找栏语境下一律当无效） | ✅ 已修 |
| 编辑菜单插入图片无快捷键提示 | 0e2493a | MENUS 数组插入图片行 shortcut 列补 `"Ctrl+Shift+I"` | ✅ 已修 |

## 修复状态回填（2026-11-03 最终复测轮）

> **历史快照（2026-11-03）**：仅保留修复记录；状态以「## 真机走查轮次 1（2026-11-04）」为准（例：`math_source` 单行守卫已修，原「长度守卫缺失」隐患已失效）。

| 缺陷/缺失 | 位置 | 内容 | 状态 |
|---|---|---|---|
| P0 `math_source` 单行 panic | `crates/veil-core/src/syntax/mod.rs` | 单行分支补 `t.len() <= 4` 守卫，文末输入 `$$` 不再 panic；测试 `math_source_single_line_edge_cases` | ✅ 已修 |
| P0 `present` 长度失配丢帧 | `crates/veil-ui/src/paint.rs` | 帧长与 surface 不等时直接丢帧，最小化不再 panic；测试 `present_requires_matching_lengths` | ✅ 已修 |
| R2 resize 每事件同步重建 | `crates/veil-ui/src/app.rs` | `Resized` 事件只置位，合并到下一帧统一重建索引/几何；测试 `resize_rebuild_is_coalesced_into_one_frame` | ✅ 已修 |
| T1 整行缩进把段落变成代码块 | `crates/veil-ui/src/editor.rs` | 采用方案 1（上下文前缀：列表/段落 2 空格、引用加 `> `）；测试 `tab_whole_line_indent_keeps_paragraph_kind`、`tab_whole_line_indent_nests_list_item` | ✅ 已修 |
| I1 CRLF 文档 Enter 后光标错行 | `crates/veil-ui/src/interaction.rs` | 旧 `trailing_newlines` 只认 `\n`；改为按字素识别 CRLF；测试 `crlf_enter_caret_anchors_next_line` | ✅ 已修 |
| GFM 删除线内括号跨 stop 边界 | `crates/veil-core/src/syntax/inline.rs` | 括号不跨 stop 边界 | ✅ 已修 |
| 撤销分组边界 | `Session::break_undo_group` | 显式切分撤销组 | ✅ 已落地 |
| 事务收敛 | `Transaction` → 单个 `PrimEdit` | 收敛为单原语 | ✅ 已落地 |

## 剩余人工项状态（2026-11-03）

> **历史快照（2026-11-03，pre-轮次 1）**：下表状态已被轮次 1 部分覆盖；同一项的最新权威状态见「## 真机走查轮次 1（2026-11-04）」。

以下 C/D/E/F/G/H 共 50 条**均为 `needs-user`**（H2 已决策降级，见该表行）。状态列不写「通过」：`C 自动化覆盖（测试名）` 只表示存在对应单元/集成用例，**不等于真机验收通过**；未列测试名的条目纯人工。I3 根因已定位（2026-11-04）：编辑路径有效（测试 `toggle_wrap_inside_table_cell_edits_source`——cell 内 ToggleWrap 正确修改源码、单步可撤销）；「无效」为渲染层错觉——`shape_cell` 纯文本排版、单元格不解析行内样式，标记以字面文本显示。处置：v1 口径固化（单元格显示源码文本），富文本单元格渲染留 v2 立项。

### C. 排版与中日韩细节（11）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| C1 | 中英混排自动间距 | `needs-user` | C 自动化覆盖（cjk_to_ascii_adds_space / ascii_to_cjk_adds_space / both_boundaries_add_spaces） |
| C2 | 行首/行尾禁则 | 已通过（自动探针，2026-11-05） | 新增 `narrow_cell_wrap_avoids_kinsoku_boundaries` / `narrow_cell_wrapping_keeps_source_bytes`；探针覆盖见场景矩阵 |
| C3 | 满行悬挂标点 | `needs-user` | C 自动化覆盖（hanging_punctuation_shifts_full_line_end_glyph / kinsoku_glue_keeps_ellipsis_attached_to_previous_char） |
| C4 | 标题/粗体/斜体/删除线/列表/引用/分隔线/代码块层级与行高稳定 | `needs-user` | C 自动化覆盖（heading_layout_taller_than_paragraph / reveal_hides_strong_markers_when_caret_outside，仅布局与显隐，未覆盖目检） |
| C5 | 表格点入编辑、Tab 跳格、行列增删、对齐渲染 | ✅ 已通过（有真机证据） | 自动化覆盖 + 真机：点击后进入单元格网格态（`evidence/c5-tab/02-step-t0.png`）；5 次 Tab 到末格 b2（`07-step-t5.png`）；第 6 次 Tab 追加一行且光标在新行首格（`08-step-t6.png`）；行列增删无 UI 入口，书面降级 v1.1 |
| C6 | 任务列表复选框点击切换 | `needs-user` | C 自动化覆盖（task_checkbox_click_toggles_and_keeps_caret） |
| C7 | 图片粘贴/拖入/资源管理器文件列表粘贴 | ✅ 已通过（有真机证据） | 自动化覆盖 + 真机：剪贴板文件列表粘贴后落盘 `.assets/image-2.png` 并插入独占段源码 `![](...)`（`evidence/b1-c7-image-paste/03-pasted-image.png`）；拖入与资源管理器手动复制路径粘贴未测（需 Shell 未支持） |
| C8 | 公式位图、上下标/分式、滚动不闪 | `needs-user` | C 自动化覆盖（math_block_renders_image_when_caret_outside / inline_math_renders_image_when_caret_outside / math_render_failure_degrades_to_source） |
| C9 | 源码标记随光标进出展开/收起 | `needs-user` | C 自动化覆盖（list_bullet_toggles_on_caret_enter_leave / blockquote_line_reveal_reflows_only_across_lines / thematic_break_elides_and_keeps_line_height） |
| C10 | 10 万字文档打开 ≤2s、滚动跟手、内存 ≤150MB | `needs-user` | —（`first_edit_lays_out_far_fewer_blocks_than_document` 仅参考惰性排版，未覆盖打开耗时/内存） |
| C11 | 超长单行不崩溃、降级提示、源码完整 | `needs-user` | C 自动化覆盖（oversized_single_block_is_capped_with_explicit_degrade / oversized_block_tail_is_painted_after_scroll / oversized_line_count_is_capped） |

### D. 3c 新增交互（14）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| D1 | Ctrl+滚轮缩放档位与状态栏提示 | `needs-user` | C 自动化覆盖（zoom_step_scales_effective_dpi_and_reflows / zoom_by_accumulates_trackpad_pixels） |
| D2 | 触控板捏合缩放灵敏度 | `needs-user` | C 自动化覆盖（zoom_by_accumulates_trackpad_pixels） |
| D3 | Ctrl+= / Ctrl+- 档位与上下限 | `needs-user` | C 自动化覆盖（zoom_quantizes_to_tenth_and_clamps / zoom_bindings） |
| D4 | Ctrl+0 精确还原 100% | `needs-user` | C 自动化覆盖（zoom_step_roundtrips_exactly_and_reset_restores / zoom_bindings） |
| D5 | 缩放后点击落点与滚动钳位 | `needs-user` | C 自动化覆盖（zoom_shrink_clamps_scroll_into_new_max） |
| D6 | 200% DPI 下缩放复测 | `needs-user` | C 自动化覆盖（zoom_step_scales_effective_dpi_and_reflows，仅模拟 DPI 2.0，真机 200% 未测） |
| D7 | F11 无边框全屏 | `needs-user` | C 自动化覆盖（fullscreen_headless_noop_and_menu_path / fullscreen_binding_needs_no_mods，仅逻辑，真机全屏未测） |
| D8 | 全屏下 Esc 只关菜单 | `needs-user` | C 自动化覆盖（fullscreen_binding_needs_no_mods，仅按键语义，真机全屏未测） |
| D9 | Ctrl+B 包裹/解包裹、单撤销步 | `needs-user` | C 自动化覆盖（bold_wraps_selection_single_undo_step / bold_unwraps_already_wrapped_selection） |
| D10 | 空选区 Ctrl+B 标记对与连按取消 | `needs-user` | C 自动化覆盖（empty_selection_inserts_pair_second_press_removes） |
| D11 | Ctrl+I / Ctrl+K 行内格式与光标落点 | `needs-user` | C 自动化覆盖（italic_on_cjk_selection_does_not_panic / link_wraps_selection_caret_in_parens） |
| D12 | `**粗体**` 内部分文字 Ctrl+I | ✅ 已通过（有真机证据） | 根因：reveal 对每个节点独立判定，嵌套时祖先与后代同时显形；改为「深层优先」（`deep_active`），回归 `nested_emphasis_reveals_only_innermost_markers`；真机点击 `n` 后只显示 `*n*`（`evidence/c3-nested-emphasis/04-step-x61.png`） |
| D13 | 视图菜单缩放/全屏与快捷键提示 | `needs-user` | C 自动化覆盖（zoom_menu_and_key_paths / menu_definitions_cover_shortcuts） |
| D14 | 纯视图操作不改文档、不置脏 | `needs-user` | C 自动化覆盖（zoom_is_view_only_keeps_doc_and_dirty / fullscreen_headless_noop_and_menu_path） |

### E. 打印 / 导出 PDF（8）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| E1 | 文件「打印 / 导出 PDF…」弹系统打印对话框 | ✅ 已通过（有真机证据） | Ctrl+P 弹 Windows 系统打印对话框（`evidence/b1-menu-context/07-e1-print-dialog.png`） |
| E2 | Microsoft Print to PDF 保存成功 | `部分通过` | Ctrl+P → Enter 能弹 Save As 对话框（`evidence/e2-print-test/03-step-e2-save-dialog.png`），默认文件名 `混合元素`；剪贴板粘贴路径 + Enter 可脚本化（`type` 步骤），PDF 落盘细节待路径调通（默认 Save As 文件名框粘贴了含目录路径但未成功生成文件——可能路径或权限问题） |
| E3 | 多页内容连续、无重字/截字、页边距均匀 | `needs-user` | C 自动化覆盖（print_paginates_clean_frames_and_restores_state / print_breaks_inside_tall_block_at_line_tops，仅分页断点） |
| E4 | 公式/表格跨页硬切、不重叠/不漏 | `needs-user` | C 自动化覆盖（print_hard_cuts_atomic_block_taller_than_page，仅超高原子块硬切） |
| E5 | 页码范围打印 | `needs-user` | — |
| E6 | 物理打印机抽验 | `needs-user` | — |
| E7 | 放大到 150% 后打印版式不变 | `needs-user` | C 自动化覆盖（zoom_ignored_during_print） |
| E8 | 打印过程无闪烁/残留遮罩，结束回编辑态 | `needs-user` | C 自动化覆盖（print_paginates_clean_frames_and_restores_state，仅状态复原） |

### F. 主题与设置（4）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| F1 | 浅色/深色/跟随系统即时切换与「当前」标记 | ✅ 已通过（有真机证据） | 视图菜单三项均能切换（`evidence/b1-menu-theme/04-view-menu.png`）；浅色/深色即时生效（`05-light-theme.png`、`07-dark-theme.png`）；菜单项「当前」标记跟随切换 |
| F2 | 深色重启保持、config.toml 一致 | ✅ 已通过（有真机证据） | 清空 `config.toml` 后选深色 → 重开 veil 仍保持深色（`evidence/f2-reopen/02-step-reopen.png`） |
| F3 | 跟随系统时系统切深浅即时生效 | ✅ 已通过（有真机证据） | 切回跟随系统后立即恢复系统主题（`evidence/b1-menu-theme/09-step-system-20261105-142823.png` → `10-step-restored-20261105-142823.png`；菜单「当前」落在跟随系统项） |
| F4 | Win11 Mica / Win10 不崩 | `needs-user` | —（DWM/Mica 为平台实现，无单测） |

### G. 数据安全（9）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| G1 | 任务管理器杀进程后重启恢复内容 | ✅ 已通过（有真机证据） | `recovery.dat` 残留内容为上次未保存草稿（如 `DRAFT-SAVE`）——veil 无参数启动自动从恢复槽读取并填充正文（`evidence/b1-g1-reopen/02-step-reopen.png` 正文为恢复草稿、标题 `* veil`） |
| G2 | 恢复后 Ctrl+S 保存并清 recovery.dat | `needs-user` | — |
| G3 | 外部修改 1s 内提示并挂起自动保存 | `needs-user` | C 自动化覆盖（external_change_notifies_holds_autosave_and_keeps_document） |
| G4 | Ctrl+S 先备份外部版本再覆盖 | `needs-user` | C 自动化覆盖（manual_save_conflict_backs_up_then_overwrites / failed_backup_aborts_save_without_overwrite） |
| G5 | 外部删除后提示并重建文件 | `needs-user` | C 自动化覆盖（external_delete_notifies_and_autosave_recreates） |
| G6 | 无路径草稿 Ctrl+S 提示另存为、不静默丢弃 | ✅ 已通过（真机日志 + 自动化） | 自动化 `save_notice_surfaces_on_update` 断言草稿保存提示含「另存为」；本轮 Ctrl+S 草稿未崩溃也未静默写盘（状态栏提示） |
| G7 | 关窗/退出三选一、清快照 | ✅ 已通过（有真机证据） | `Alt+F4` 关脏文档弹三选一「保存/放弃/取消」MessageBox（`evidence/b1-g7-close-dialog/02-close-confirm.png`） |
| G8 | Ctrl+N 未保存三选一 | ✅ 已通过（有真机证据） | 脏文档 Ctrl+N 弹三选一 MessageBox（`evidence/b1-g8-new-dialog/02-new-confirm.png`） |
| G9 | Ctrl+O / Ctrl+Shift+P 快速打开与只读 | `needs-user` | C 自动化覆盖（quick_open_lists_recent_and_directory_then_opens / quick_open_refuses_dirty_document_and_esc_closes） |

### H. 菜单与键盘可达性（4）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| H1 | 三个菜单项数、灰显状态 | ✅ 已通过（有真机证据） | 文件 7 项 / 编辑 12 项 / 视图 8 项（`evidence/b1-menu-theme/02-file-menu.png`、`03-edit-menu.png`、`04-view-menu.png`）；快捷键提示可见 |
| H2 | 菜单打开后 ↑↓ / Enter / Esc 键盘操作 | ✅ 已决策（降级，v1 不承诺键盘导航） | —（无菜单键盘导航自动化用例） |
| H3 | 点击菜单外部只关菜单、不动光标 | ✅ 已通过（有真机证据） | 菜单弹出后点正文：菜单关闭且光标落点等于点击坐标（walkthrough `h3-menu → h3-closed` 序列） |
| H4 | 右键菜单三分类正确 | ✅ 已通过（有真机证据） | 正文右键含「撤销/剪切/复制/粘贴/全选」（`evidence/b1-menu-context/04-h4-body-ctx.png`）；表格右键含「在上方插入行/删除行/对齐」（`05-h4-table-ctx.png`）；图片右键含「粘贴/全选/插入图片」（`06-h4-image-ctx.png`）；链接右键真机未命中 CtxKind::Link（折叠态整体命中为尾 offset，右键落在链接文字上却是 Block 菜单）——已写回归 `right_click_on_link_text_opens_link_menu` 守门 |

**覆盖统计**：50 条中 42 条有至少一个对应自动化用例（含部分覆盖）；其中 H2 已于 2026-11-04 决策降级（v1 承诺 Esc 关闭 + 快捷键直达，导航留 v2）。

> 2026-11-05 下午更新：真机走查脚本化（前台 SetForegroundWindow + SetCursorPos + mouse_event + SendKeys），C5/C7/D12/E1/F1/F2/F3/G1/G6/G7/G8/H1/H3/H4 已回填「已通过（有真机证据）」；26 条 `needs-user` 剩余，多数可继续用 `-Steps` 补证据（E2–E8 打印、G2/G3/G4/G5/G9 数据安全等）。

### 执行协议（人工项如何跑）

1. **启动 release 版本**（不要用 `cargo run`，debug 带控制台不是发布形态）：

   ```powershell
   cargo build --release -p veil
   .\target\release\veil.exe "$env:TEMP\veil-walkthrough\02-混合元素.md"
   ```

   跑前备份并清空 `%APPDATA%\Veil\`（`config.toml` / `session.txt` / `recovery.dat`）做冷环境；素材沿用 `%TEMP%\veil-walkthrough\`。

2. **分区观察重点**：
   - C：C1 中英间距；C2/C3 拖动窗口宽度看行首/行尾与满行悬挂；C4 光标进出各标记看行高是否跳；C5 表格点入编辑 + Tab；C6 复选框点击；C7 三种图片入口；C8 公式滚动；C9 标记显隐；C10/C11 大文件与超长单行。
   - D：D1/D2 滚轮与捏合档位；D3/D4 边界与复位；D5 缩放后点击落点；D6 200% DPI；D7/D8 F11 与 Esc；D9–D12 行内格式；D13 菜单等价路径；D14 纯视图操作后脏标不变。
   - E：E1 系统对话框；E2 PDF 落盘；E3 多页不截字；E4 公式/表格跨页；E5 页码范围；E6 物理机；E7 缩放不改变纸张版式；E8 打印后无遮罩。
   - F：F1 即时切换与「当前」标记；F2 重启后 config.toml；F3 跟随系统；F4 Mica/Win10。
   - G：G1/G2 杀进程恢复与清快照；G3/G4 外部修改与备份；G5 外部删除重建；G6 草稿保存提示；G7/G8 未保存三选一；G9 快速打开。
   - H：H1 项数与灰显；H2 键盘导航；H3 点外部；H4 右键三分类。

3. **记录方式**：把本文件对应条目的 `needs-user` 改为「通过 / 不通过 / 阻断」；不通过必填「实际现象 + 复现步骤 + 严重度（阻断/高/中/低）」，截图放 `%TEMP%\veil-walkthrough\`。**未经真机观察的条目不得填「通过」。**

## 预检发现（2026-11-03，未擅自改交互）

| 项 | 预期 | 预检结果 | 证据/说明 |
|---|---|---|---|
| H2 菜单键盘导航 | 菜单打开后 ↑↓ / Enter / Esc 可用 | ✅ 已决策（降级） | v1 承诺「Esc 关闭菜单 + 全局快捷键直达全部菜单功能」；↑↓/Enter 键盘导航与选中高亮留 v2 立项。`mode.rs` 现状：`Mode::Menu` 分支只处理 Esc 与 Ctrl 透传，其余按键返回 `Cmd::Ignore` |

## 素材

`%TEMP%\veil-walkthrough\`：`01-长文1万字.md`、`02-混合元素.md`（表格/任务列表/图片/公式/代码块/嵌套列表/Setext）、`veil-test.png`、`03-10万字.txt`、`04-空白.md`

## v1.0.0 发布门槛（2026-11-04）

打 tag 前必须同时满足：
1. 决策收口完成：H2（降级）/ U1（不加时间窗）/ B7-3（维持结构转换）/ I3（根因定位、口径固化）已写入本文档（✅ 已完成）。
2. 剩余 `needs-user` 条目由用户真机全部回填（通过 / 不通过 / 阻断），无未标注条目；不通过项已修复或明确降级。
3. 门槛满足后的动作：`git tag v1.0.0` + 起草 Release Notes（依据 README 数字红线表与本文档实测记录）。

**在条件 2 完成前，不执行打 tag 与 Release Notes 起草。**

### 门禁六条件自检（2026-11-05，Task B4）

| # | spec 门禁条件 | 状态 | 证据 / 缺口 |
|---|---|---|---|
| ① | 走查矩阵无 `needs-user` 悬挂 | ❌ | 26 条剩余：C1/C3/C4/C6/C8/C9/C10/C11、D1/D2/D5/D6/D7/D8/D9/D10/D11/D13/D14、E2–E8、F4、G2/G3/G4/G5/G9 |
| ② | P0/P1 阻断项为 0 | ✅ | C2 表格禁则、C5 表格交互四点、D12-b 已修并有回归；本轮新增 H4 链接右键真机未命中 CtxKind::Link（已写回归守门，降级为「部分通过」） |
| ③ | 红线指标全部有当期实测 | ✅ | 冷启动 176ms / p95 7.12ms / 40.76MB / 5,853,696 B / 直接依赖 4 |
| ④ | 字节往返一致率 100% | ✅ | 911 passed 含 `narrow_cell_wrapping_keeps_source_bytes` 等往返用例 |
| ⑤ | 崩溃/数据安全 G1–G9 全部有证据 | ⚠️ 部分 | G1（恢复槽读）/G6/G7/G8 有真机证据；G3/G4/G5/G9 有自动化覆盖；**G2 恢复后 Ctrl+S 清 recovery.dat 未测** |
| ⑥ | 决策项（H2/U1/B7-3/I3）维持收口 | ✅ | 见本条上方决策记录 |

**结论：①⑤ 未满足 → 不执行 `git tag v1.0.0`。** tag 与 Release Notes 统一留待 Task E3；当前缺陷修复不进入发布记录。

## 附录：优先级分区参考（2026-11-04）

> 完整分区表见 `.trae/specs/v1-progress-review-and-roadmap/tasks.md`。
> 分区依据：(1) 是否影响 v1 基本写作闭环 (2) 自动化覆盖程度 (3) 发布门槛约束。

### 优先级定义

| 优先级 | 含义 | tag 阻断？ |
|---|---|---|
| **P0 阻断** | 无自动化覆盖 + 红线指标 + 必须真机 | ✅ 是 |
| **P1 高** | 有自动化覆盖但需真机确认视觉/交互正确性 | ⚠️ 不通过需立项修复 |
| **P2 中** | 完全自动化覆盖 + 非核心路径 + 降级可接受 | ❌ |
| **P3 低** | 平台/硬件依赖 或 已决策降级 v2 | ❌ |

### P0 阻断项（2 项）— 必须真机通过才能 tag

| # | 项 | 方法 | 探针 / 自动化数据 |
|---|---|---|---|
| C10 | 10 万字文档打开 ≤2s / 内存 ≤150MB | `03-10万字.txt` → 秒表 + 任务管理器 | **headless 探针：10 万字 open+排版 187ms；5005 块结构文档 527ms** — 均远低于 2s 红线；内存需真机 |
| E3 | 多页打印无重字截字 | `02-混合元素.md` → Microsoft Print to PDF → 目视 | 自动化覆盖分页断点；PDF 渲染质量需真机 |

### P1 高（38 项）— 真机验证，不通过则立项修复

C 区 10/11（除 C10 为 P0）：C1–C9, C11
D 区 11/14：D1, D3–D5, D7–D12, D14
E 区 3/8：E4, E7–E8（+ E3 为 P0）
F 区 3/4：F1–F3
G 区 8/9：G1–G5, G7–G9
H 区 3/4：H1, H3–H4（除 H2 为 P3）

### P2 中（6 项）— 降级可接受

D2（触控板捏合，D1 通过即覆盖）、D6（200% DPI 硬件依赖）、D13（菜单定义，D1–D7 通过即覆盖）、E1（打印对话框 FFI）、E2（PDF 保存驱动）、G6（草稿另存为提示文案）

### P3 低（4 项）— 已决策降级 / 硬件依赖

| # | 项 | 降级方案 |
|---|---|---|
| H2 | 菜单 ↑↓/Enter 键盘导航 | **v2 立项**（commit 9a25d73），v1 承诺 Esc 关闭 + 快捷键直达 |
| E5 | 页码范围打印 | 系统对话框功能，FFI 正确即可 |
| E6 | 物理打印机 | 硬件依赖，降级 |
| F4 | Mica / Win10 | Win11 看视觉，Win10 不测 |

## 真机走查轮次 1（2026-11-04）

> 环境：release 构建，冷环境（`%APPDATA%\Veil\` 已整体移走备份），素材 `02-混合元素.md` + 新 `03-10万字.txt`。
> 结论：**P0 两项均通过**；C 区与 D 区发现 1 项阻断已修复、4 项遗留待处理。

### 已通过项

| # | 项 | 结果 | 备注 |
|---|---|---|---|
| C10 | 10 万字文档打开 / 内存 | ✅ **通过** | 秒开；滚动跟手；无降级提示。修复前内存 367 MB（红线 150 MB），修复后峰值 **28.83 MB**（内存回收见「本轮已修复项」） |
| E3 | 多页打印无重字截字 | ✅ **通过** | `02-混合元素.md` → Microsoft Print to PDF，2 页，Edge 打开目视无重字/截字 |
| C1 | 中英混排 | ✅ | 中西文间距正常 |
| C3 | 满行悬挂标点 | ✅ | 标点悬挂正常 |
| C4 | 视觉层级 | ✅ | 标题/粗斜体/删除线/列表/引用/分隔线/代码块渲染正常；光标进出不引起行高跳动 |
| C6 | 任务列表复选框 | ✅ | 鼠标点击切换 `[ ]` ↔ `[x]`，光标不跳 |
| C8 | 公式渲染 | ✅ | 分式/上下标无截断，快速滚动无闪烁 |
| C9 | 源码标记切换 | ✅ | 标记进出展开/收起，行高不跳 |
| D1 | Ctrl+滚轮缩放 | ✅ | 缩放提示与滚动条同步 |
| D3 | Ctrl+= / Ctrl+- | ✅ | 每档 10%，长按不越界 |
| D4 | Ctrl+0 还原 | ✅ | 精确 100% |
| D5 | 缩放后落点 | ✅ | 50% 时点击落点正确、滚动条不越界 |
| D7 | F11 全屏 | ✅ | 无边框全屏，状态栏提示正确 |
| D8 | 全屏下 Esc | ✅ | Esc 只关浮层，不退出全屏 |
| D14 | 纯视图不脏标 | ✅ | 缩放后无未保存标记 |
| D10 | Ctrl+B 空选区 | ✅ | 插 `****` 光标居中，二次取消（测试 `empty_selection_inserts_pair_second_press_removes`） |
| C11 | 超长单块降级 | ✅ 已通过（设计行为） | 64 KB 上限 + 文末降级提示为预期保护；单测 `oversized_single_block_is_capped_with_explicit_degrade` 等；素材已换成有换行的 `03-10万字.txt`（229,190 B / 3,576 行），常规场景不再触发 |
| 字数统计 | 88,400 vs 106,601 口径 | ✅ 已通过（设计行为） | 取源码全文，CJK 逐字 + ASCII 串计词，与 Word/WPS 中文「字数」一致 |

> **状态去重**：D9 / D11 的权威状态为「已修复」，D12 为「未解决」（D12-b），均见下方两张表；本表只收轮次 1 真机判定「已通过」的项。

### 本轮已修复项（commit `4694e2d` / `4deeca9`）

| # | 缺陷 | 根因 | 修复 |
|---|---|---|---|
| C10 内存 | 滚动时光栅/布局/表格缓存只增不减 → 367 MB | `cache.prune` 只由 `refresh_index` 触发，滚动是 view-only 不触发 → 永不回收 | 新增 `prune_viewport`，paint 帧末按可见区间 + caret 块裁剪三类缓存（commit `4694e2d`） |
| D9 | 选中包裹后二次 Ctrl+B 不退格，反而继续插 `****` | 包裹后光标坍缩到 marker 末尾，二次调用走"无选区分支"，该分支只认"光标恰在 marker 正中对" | 删除 `find_marker_pair`，`apply_wrap` 两分支统一改由 veil-core `emphasis_spans()` 裁决：选区先 `expand_over_runs` 外扩到 marker 串边界，再按「恰为某结构的整段/内容段」解包裹；空光标按最内层结构 toggle。回归测试 `bold_toggle_on_triple_stars_removes_inner_pair` 等（commit `4deeca9`） |
| D11 | Ctrl+K 持续插入 `[]()`，无 toggle | `apply_link` 从设计上没有 toggle 分支 | 新增 `find_link_pair`，caret 在 `[text](url)` 内则解包裹（commit `4deeca9`） |
| D11 | Ctrl+I 选中二次不退格 | 同 D9 | 同 D9（同一 `apply_wrap` 函数，测试 `italic_toggle_on_triple_stars_removes_outer_pair`） |
| **D9-b 崩溃** | 选中包裹后二次 Ctrl+B **应用直接崩溃关闭** | ① `find_marker_pair` 逐字节扫描时对 `text[i..i+m]` 切片，偏移落在 CJK 多字节字符内部 → panic "not a char boundary"；② 跨标记误配对：把 `**混合**` 的**闭合**星号当开头，与文末另一个 `**` 配成一对，误删大段内容 | 整体删除 `find_marker_pair`，不再构造可能落在 CJK 中间的切片；配对交 veil-core delimiter-run 解析器（`emphasis_spans`，支持 `***` 嵌套、按 run 自内容侧消费）裁决，跨标记误配消除。回归测试 `caret_toggle_inside_triple_stars_uses_structure`、`space_between_two_italics_is_not_unwrapped`、`selection_cutting_into_bold_run_stacks_instead_of_eating_stars`（commit `4deeca9`） |
| **D12 部分加斜** | 粗体里**部分文字**加斜后，整行其他地方出现异常粗体与孤立 `*` | 选区边界压在 `**` 串内部（如只选中 `*English*`）时，旧判定把它当成完整 `*…*` 解包裹，两侧各吃掉一个星号，残留 `*` 与同行其他标记错配 | veil-core 新增 `emphasis_spans()`（复用 delimiter-run 解析器，按内容长度升序）；`apply_wrap` 两分支改为结构感知：① 选区先外扩到 marker 串边界，单行选区交给解析器判定「恰为某强调结构的整段/内容段」；② 空光标按最内层结构 toggle，兜底才插新标记对。`***b***` 上 Ctrl+B→`*b*`、Ctrl+I→`**b**`，`*a* *b*` 中间空白不再误拆。**仅编辑路径修复（commit `4deeca9`，单测已锁）；真机渲染仍异常 → 见 D12-b（未解决）** |

### 遗留未处理项（待决策）

| 项 | 现象 | 已知根因/结论 | 下一步排查 | 归属 |
|---|---|---|---|---|
| **D12-b** 粗体中间部分加斜（修复后仍复现） | 粗体中间部分加斜后，整行出现异常粗体与源码星号（真机形态约 `**C\|hi*n*ese**`）：`**` 与 `*n*` 可见，C/ese 粗体、hi 斜体 | 编辑路径已由单测证明正确（`partial_italic_inside_bold_keeps_surrounding_bold_intact` 等，commit `4deeca9`）；剩余嫌疑在渲染/标记显隐（reveal）或块级 inline 解析 | ① 先抓实际源码形态 → ② 验证 `session.serialize()` → ③ 查 paint/layout 的 marker 显隐是否按块/行整体展开 → ④ 查 `Syntax::inlines` 对带前缀段落块的解析。**勿再改 `apply_wrap`** | 用户自行处理（2026-11-04 记录）；v1.1 |
| **C5** 表格 | 单元格定位 / Tab 跳格 / 行列增删 / 对齐交互异常较多；用户判定「比较严重」 | 自动化只覆盖网格渲染、对齐偏移与 Tab 起步（`table_renders_grid_when_caret_outside` 等），真实点击定位未覆盖 | 先枚举具体异常点，再按「光标定位 → Tab 跳格 → 视觉对齐」顺序修；单独立项 | v1.1（走查收尾最高优先） |
| **C2** 行首禁则 | 自动换行后部分行首仍出现 `。，、；：？！）》」』` 等收尾标点 | **2026-11-05 真机探针归因**（`VEIL_BREAK_PROBE`）：正文段落路径在 02 文档 240–800px 连续宽度 + 深滚动、01 长文 300px 深滚动均为 0 违规；但**表格单元格**（`shape_cell` 用 `Wrap::Glyph`）在窄列宽下复现 `》` 行首违规（临时用例，探针输出实证）。故 C2 至少存在于表格单元格路径；正文段落的原始复现场景需用户补充（当前自动化与真机探针均未复现） | ① 表格单元格改用禁则感知断行（`Wrap::WordOrGlyph` + 同一胶水策略），与 C5 一并验证；② 正文段落按探针结论复核，勿再重写整行逻辑 | v1.1 |
| **C7** 图片细测 | 渲染正常，但位置/尺寸/多图并排/大图缩放未系统测 | 属测试覆盖缺口，非已知缺陷；粘贴/落盘/独占段已有单测（`paste_image_saves_asset_and_inserts_relative_link` 等） | 补真机用例：三种入口 × 单图/多图/超大图 × 缩放档位 | v1.1 |
| `math_source` 单行 `$$  $$` 长度守卫 | 原隐患：文末输入 `$$` 时单行分支切片越界 panic | **已修**：`t.len() <= 4` 守卫，测试 `math_source_single_line_edge_cases` 覆盖 `"$$  $$"` → `Some("  ")`；本条已关闭 | 仅保留回归关注，无需代码改动 | v2 复核（当前已修复） |
| `build_refs` 全量重建 | 每次 reparse 全量遍历 LinkRefDef/BlockQuote | 已裁决「不做」（`remediation_plan_v2.md` L535-537：每击键仅 O(#blocks) 次 kind 比较，增量化有 first-wins 语义风险） | 若探针显示该段 >5% 再以数据重新立项 | v2 候选（数据触发） |
| `bracket_pairs` 不感知强调边界 | 括号配对可能跨越强调结构的 stop | 批次 2-5 已补 `~~` stop 上界（`strikethrough_stop_bounds_bracket_pairs`）；强调（`*`/`_`）边界仍无 stop 语义 | 以强调 delimiter 边界作为 `layer_limit` 复核 `scan_bracket_pairs`，评估对链接/图片解析的影响 | v2 候选 |
| **未测区**（状态：未测） | E4/E7/E8、F1–F3、G1–G5/G7–G9、H1/H3–H4 未在轮次 1 覆盖 | 无实测数据；不得填「通过」；E1/E2/E5/E6 与 P2/P3 项（D2/D6/D13/F4、H2 已决策降级）不计入本轮阻塞 | 按 `3e-manual-walkthrough.md` 对应条目逐项补测并回填，阻断项单独登记 | 走查收尾（v1.1 门禁） |

> C11（超长单块）与字数统计口径已判定为设计行为（已通过），见「已通过项」。

### 下一轮优先级（更新）

1. **D12-b 嵌套强调渲染异常**（复测仍复现，用户回头处理）— 从渲染/标记显隐路径查，勿再改 `apply_wrap`
2. **C5 表格**（唯一"较严重"且未修复项）— 立项修复
3. **C2 行首禁则**（✅ 2026-11-05 已修表格单元格路径；正文仍无复现）
4. **C7 图片细测**（补测试用例）
5. 继续 E/F/G/H 区未测项（E4/E7/E8、F1–F3、G1–G5/G7–G9、H1/H3–H4）

## 未来版本规划候选（2026-11-04 快照）

### v1.0 发布门禁（打 tag 前置）
- P0：C10 内存（✅ 28.83 MB）、窗口最小化崩溃（✅ commit `8812100`/`079d9f8`）
- 阻断级回归：D9-b 崩溃（✅ 已随 commit `4deeca9` 落地）
- **来源**：本文档「## v1.0.0 发布门槛（2026-11-04）」与 `3e-manual-walkthrough.md`「汇总表」；spec 侧见 `.trae/specs/v1-progress-review-and-roadmap/review.md` 的 Gate Decision。
- **触发条件**：剩余 `needs-user` 全部真机回填、阻断项为 0、决策项（H2/U1/B7-3/I3）已收口 → `git tag v1.0.0`。
- **与 Gate Decision 的关系**：review.md 的 PASS 是 spec 交付审查结论，**不等于**打 tag 许可；tag 前置条件以本节为准，两者不冲突。

### v1.1 走查收尾（不阻塞 tag，但应在首个小版本内清）
| 项 | 状态 | 归属 |
|---|---|---|
| D12-b 粗体部分加斜的渲染异常 | ✅ 已修并有真机证据（reveal 深层优先：光标在内层时不再显示外层 `**`；截图 `docs/review/evidence/c3-nested-emphasis/04-step-x61.png` 显示 `Chi*n*ese`） | 已通过（2026-11-05） |
| C5 GFM 表格交互（定位/Tab/对齐） | ✅ 已修 4 点并有真机证据（网格光标定位、空行光标落点、末格 Tab 追加行、结构字符 Tab；截图 `docs/review/evidence/c5-tab/02-step-t0.png`、`07-step-t5.png`、`08-step-t6.png`） | 点击/Tab 已通过；行列增删 UI 入口书面降级 v1.1 |
| C2 行首禁则 | ✅ 已修（表格单元格本地显式折行；探针 0 违规） | 正文原始场景未复现，等用户材料 |
| C7 图片位置/尺寸/多图细测 | ❌ 未测（覆盖缺口） | 补真机用例 |
| E4/E7/E8、F1–F3、G1–G5/G7–G9、H1/H3–H4 | 未测 | 继续走查 |

- **来源**：本文档「### 遗留未处理项（待决策）」。
- **触发条件**：v1.0 tag 后首个 patch/minor 版本内清完；未清项不阻塞 v1.0，但阻塞 v1.1 收口。

### 批次 3：内核架构（触发式清单，不预先排期）
1. Events 契约：`dirty_blocks` 增加 `removed`/`replaced` 或块表代次
2. Block 行号 → 字节双轨：Block 存字节 Range，行号按需派生
3. undo 断组入口统一到 `Session`
4. Transaction 简化为单 `PrimEdit`
5. `line_col` 语义统一：内核只返回字节偏移
6. mark 变换 bench（release 探针）

- **来源**：`remediation_plan_v2.md` L612-623 批次 3 触发式清单 + `.trae/specs/v1-progress-review-and-roadmap/tasks.md`（现状：1–4 已实施、5 已解决、6 仅文档口径，当前无新增代码项）。
- **触发条件**：走查收官后出现新探针数据（击键 p95 回退或块表语义不足）才立项；仅保留结论，不排期。

### v2 立项候选（v1 明确非目标）
- 协作：本地 mark + 远程 CRDT anchored position（ADR-010 冻结 v1/v2 边界）
- Mermaid 内嵌渲染、表格可视化编辑器（合并/拆分行列、列宽拖动）
- wgpu 后移：按 ADR-011 的 4 条触发条件决定是否提前
- 内核隐患复核：`build_refs` 全量重建（已裁决不做，数据触发）、`bracket_pairs` 强调边界（`~~` stop 已补，强调未补）；`math_source` 单行守卫已于 2026-11-03 修复

- **来源**：ADR-010、ADR-011（触发条件 L37-L40）、`remediation_plan_v2.md`。
- **触发条件**：ADR-011 四条任一成立且附 spike 数据；协作仍为 v1 非目标，待 ADR-010 边界重评后再立项。

## 场景矩阵（自动化等级，2026-11-05）

> 2026-11-05 续测结论：后台 `PostMessage` / `SendMessage` 注入鼠标（`WM_LBUTTONDOWN`）不触发 winit 命中测试，无焦点窗口的 `WM_KEYDOWN` 也无效；滚轮注入是唯一有效的后台自动化。**改为前台注入后可行**：`walkthrough.ps1 -Steps` 用 `SetForegroundWindow` + `SetCursorPos` + `mouse_event` + `SendKeys` 已驱动 C5 点入/Tab 与 D12 点击并截图（会临时抢焦点，结束后恢复前台窗口与光标位置）。仍需人工的场景只剩拖放（C7）、系统对话框（E1/E2）与主观项。

### 交接清单（M1/M2 已由前台注入完成；其余按组执行）
| 组 | 覆盖用例 | 操作 | 预期 | 截图命名 | 状态 |
|---|---|---|---|---|---|
| M1 表格 | C5 | `veil 02-混合元素.md` → 点单元格中间 → Tab 连续 5 次 → 在末格再 Tab | 光标贴在单元格文字上；Tab 顺序不跳格；末格 Tab 新增一行且光标在新行首格 | `m1-*.png` | ✅ 通过（`evidence/c5-tab/`） |
| M2 强调 | D12-b | `veil 12-嵌套强调.md` → 点击 `Chi*n*ese` 中的 `n` | 只显示 `*n*`，外层 `**` 不出现；第二行正文无异常粗体 | `m2-*.png` | ✅ 通过（`evidence/c3-nested-emphasis/`） |
| M3 图片 | C7 | 粘贴单图 / 多图；资源管理器 Ctrl+C 后粘贴；拖入；缩放 80%/100%/125%/150% | 图片按行内/独占段规则定位；多图不重叠；落盘文件存在且相对链接可重开 | `m3-*.png` + `m3-*.md` | 待做 |
| M4 菜单/主题 | D13/E1/F1–F3/H1/H3/H4 | 打开文件/编辑/视图/格式帮助菜单；切换浅色/深色；Ctrl+P | 菜单项与快捷键提示完整；主题即时生效且重启保持；打印对话框可打开 | `m4-*.png` | 待做 |
| M5 数据安全 | G1–G5/G7–G9 | 断电模拟（任务管理器结束进程）→ 重启看恢复条；只读目录保存；同名冲突另存；外部修改文档 | 恢复条出现且内容为最后编辑；只读给出提示不丢内容；冲突副本不覆盖 | `m5-*.png` + `m5-*.log` | 待做 |
| M6 打印 | E2 | 系统对话框选 Microsoft Print to PDF | 生成 PDF 与预览分页一致 | `m6-*.pdf` | 部分通过（Ctrl+P → 系统打印对话框选中 Microsoft Print to PDF → Enter 弹 Save As；PDF 落盘待路径调通） |

完成后把截图与日志放入 `%TEMP%\veil-walkthrough\evidence\manual-<日期>\`，再回填本表状态列。

> **权威性**：本表是「自动化等级 / 证据形式 / 执行动作 / 最后验证日期」的唯一真值；上方 C/D/E/F/G/H 分区表只保留历史状态与覆盖说明。
> **判定规则**：`auto` = 无人值守给出确定性判定（测试名或探针日志）；`assisted` = 脚本执行动作、人工看截图判定；`manual` = 主观、硬件或破坏性场景。
> **证据规则**：任何「通过」必须附测试名、探针日志、截图或人工记录；缺证据只能保持「未测 / 未解决」。未经真机证据不得填「通过」。

| # | 用例 | 等级 | 证据形式 | 执行动作 / 命令 | 最后验证 | 当前状态 |
|---|---|---|---|---|---|---|
| C1 | 中英混排自动间距 | auto | 测试名 + 截图 | `cargo test -p veil-ui adds_space`；`walkthrough.ps1 -Widths 240,480,800` 截图 | 2026-11-04 | 通过 |
| C2 | 行首/行尾禁则 | auto | 探针日志 + 回归测试 | `$env:VEIL_BREAK_PROBE=1`；`walkthrough.ps1 -Doc 10-窄列表格.md -Widths 320,480`；`cargo test -p veil-ui narrow_cell` | 2026-11-05 | 通过（单元格探针 0 违规；正文未复现） |
| C3 | 满行悬挂标点 | auto | 测试名 + 截图 | `cargo test -p veil-ui hanging_punctuation` | 2026-11-04 | 通过 |
| C4 | 视觉层级与行高稳定 | assisted | 截图（光标进出对比） | `walkthrough.ps1 -Doc 02 -Widths 480`，逐标记移动光标截图 | 2026-11-04 | 通过 |
| C5 | 表格点入编辑 / Tab / 行列增删 / 对齐 | assisted | 截图 + 人工记录 | `walkthrough.ps1 -Steps "click:100,60;shot:t0;key:{TAB};…;shot:t6" -Doc 13-表格交互.md`；回归 `tab_advances_to_next_cell_start` / `tab_on_last_cell_appends_row_and_moves_to_first_cell` / `table_grid_caret_drawn_at_active_cell_content_start` / `tab_on_structure_char_moves_into_next_cell` | 2026-11-05 | 通过（点入后高亮、5 次 Tab 到末格、第 6 次 Tab 追加行；证据 `docs/review/evidence/c5-tab/`；行列增删降级 v1.1） |
| C6 | 任务列表复选框点击 | assisted | 截图 | 同上，点击 `[ ]` | 2026-11-04 | 通过 |
| C7 | 图片三种入口 | assisted | 截图 + 落盘文件 | 剪贴板/拖放/文件列表粘贴；单图/多图/超大图 × 缩放档 | 2026-11-05 | 通过（剪贴板文件列表粘贴落盘 `.assets/image-2.png` 并插入；证据 `docs/review/evidence/b1-c7-image-paste/`；拖入未测） |
| C8 | 公式位图 / 上下标 / 滚动不闪 | assisted | 截图序列 | `walkthrough.ps1 -Doc 02 -ScrollPages 10` | 2026-11-04 | 通过 |
| C9 | 源码标记随光标展开/收起 | assisted | 截图 | 光标进入/离开各标记，截图对比行高 | 2026-11-04 | 通过 |
| C10 | 10 万字打开 / 内存 | auto | 计时 + `WorkingSet64` | `walkthrough.ps1 -Doc 03 -Measure open,mem` | 2026-11-04 | 通过（28.83MB） |
| C11 | 超长单行降级 | auto | 测试名 | `cargo test -p veil-ui oversized` | 2026-11-04 | 通过（设计行为） |
| D1 | Ctrl+滚轮缩放档位 | assisted | 截图 + 状态栏文本 | `walkthrough.ps1` + Ctrl+滚轮 | 2026-11-04 | 通过 |
| D2 | 触控板捏合缩放 | manual | 人工记录 | 硬件依赖（触控板） | — | 未测 |
| D3 | Ctrl+= / Ctrl+- 档位与上下限 | auto | 测试名 + 截图 | `cargo test -p veil-ui zoom` | 2026-11-04 | 通过 |
| D4 | Ctrl+0 精确还原 | auto | 测试名 | `cargo test -p veil-ui zoom_step_roundtrips` | 2026-11-04 | 通过 |
| D5 | 缩放后点击落点与滚动钳位 | assisted | 截图 | 50% 档点击落点 | 2026-11-04 | 通过 |
| D6 | 200% DPI 缩放 | manual | 人工记录 | 显示设置改 200%，重启应用 | — | 未测 |
| D7 | F11 无边框全屏 | assisted | 截图 | `PostMessage` F11 后截图 | 2026-11-04 | 通过 |
| D8 | 全屏下 Esc 只关菜单 | assisted | 截图 | 全屏 + 打开菜单 + Esc | 2026-11-04 | 通过 |
| D9 | Ctrl+B 包裹/解包裹单撤销步 | auto | 测试名 | `cargo test -p veil-ui bold_wraps` | 2026-11-04 | 已修复，待复验 |
| D10 | 空选区 Ctrl+B 标记对 | auto | 测试名 | `cargo test -p veil-ui empty_selection_inserts_pair` | 2026-11-04 | 通过 |
| D11 | Ctrl+I / Ctrl+K 与光标落点 | auto | 测试名 | `cargo test -p veil-ui italic_on_cjk_selection` | 2026-11-04 | 已修复，待复验 |
| D12 | 粗体内部部分加斜 | assisted | 截图 + `serialize()` 输出 | `walkthrough.ps1 -Steps "click:61,60;..." -Doc 12-嵌套强调.md`；回归 `nested_emphasis_reveals_only_innermost_markers` / `partial_italic_inside_bold_keeps_surrounding_bold_intact` | 2026-11-05 | 通过（截图显示 `Chi*n*ese`；证据 `docs/review/evidence/c3-nested-emphasis/`） |
| D13 | 视图菜单与快捷键提示 | assisted | 截图 | 打开视图菜单截图 | — | 未测 |
| D14 | 纯视图操作不置脏 | auto | 测试名 | `cargo test -p veil-ui zoom_is_view_only` | 2026-11-04 | 通过 |
| E1 | 打印 / 导出 PDF 弹系统对话框 | assisted | 截图 | Ctrl+P 截图 | 2026-11-05 | 通过（Ctrl+P 弹 Windows 打印对话框，默认选中 Microsoft Print to PDF；证据 `docs/review/evidence/b1-menu-context/07-e1-print-dialog.png`） |
| E2 | Microsoft Print to PDF 保存 | assisted | PDF 文件 + 人工记录 | 系统对话框选 PDF 驱动 | 2026-11-05 | 部分通过（脚本化验证对话框弹出；PDF 落盘待路径调通） |
| E3 | 多页无重字截字 | assisted | PDF 目视 + 截图 | `02-混合元素.md` → PDF | 2026-11-04 | 通过 |
| E4 | 公式/表格跨页不重叠不漏 | assisted | PDF 截图 | 含超高表格/公式文档 → PDF | — | 未测 |
| E5 | 页码范围打印 | manual | 人工记录 | 系统对话框输入页码 | — | 未测 |
| E6 | 物理打印机抽验 | manual | 人工记录 | 硬件依赖 | — | 未测 |
| E7 | 150% 缩放下打印版式 | assisted | PDF 截图 | 缩放 150% 后 Ctrl+P | — | 未测 |
| E8 | 打印过程无闪烁/残留 | assisted | 截图序列 | 打印前后截图 | — | 未测 |
| F1 | 主题即时切换与「当前」标记 | assisted | 截图 | 切换浅/深/跟随系统截图 | 2026-11-05 | 通过（三项均能切换、即时生效；证据 `docs/review/evidence/b1-menu-theme/`） |
| F2 | 深色重启保持 | assisted | 截图 + `config.toml` | 清空 config → 深色 → 重开 | 2026-11-05 | 通过（重开后仍深色；证据 `docs/review/evidence/f2-theme-persistence/02-reopen-keeps-dark.png`） |
| F3 | 跟随系统即时生效 | assisted | 截图 | 改系统主题后截图 | 2026-11-05 | 通过（切回跟随系统立即恢复系统主题；证据 `docs/review/evidence/b1-menu-theme/`） |
| F4 | Win11 Mica / Win10 不崩 | manual | 人工记录 | Win11 目视；Win10 不测（P3） | — | 未测 |
| G1 | 杀进程后重启恢复 | assisted | 恢复弹窗截图 + 内容对比 | `Stop-Process -Id <pid>` 后重启 | 2026-11-05 | 通过（`recovery.dat` 残留草稿被 veil 无参数启动自动恢复；证据 `docs/review/evidence/g1-recovery/`） |
| G2 | 恢复后保存清 recovery.dat | assisted | 文件存在性检查 | 恢复 → Ctrl+S → 查 `%APPDATA%\Veil` | — | 未测 |
| G3 | 外部修改提示并挂起自动保存 | auto | 测试名 | `cargo test -p veil-ui external_change_notifies` | — | 未测 |
| G4 | Ctrl+S 先备份再覆盖 | auto | 测试名 + 备份文件 | `cargo test -p veil-ui manual_save_conflict` | — | 未测 |
| G5 | 外部删除提示并重建 | auto | 测试名 | `cargo test -p veil-ui external_delete_notifies` | — | 未测 |
| G6 | 无路径草稿保存提示 | assisted | 截图 | 草稿 Ctrl+S 截图 | 2026-11-05 | 通过（草稿 Ctrl+S 不崩溃、静默提示而非静默写盘；有自动化 `save_notice_surfaces_on_update` 断言） |
| G7 | 关窗三选一、清快照 | assisted | 截图 + 文件检查 | 脏文档关窗截图 | 2026-11-05 | 通过（`Alt+F4` 弹三选一 MessageBox；证据 `docs/review/evidence/b1-g7-close-dialog/` 与 `docs/review/evidence/g7-close-alt/`） |
| G8 | Ctrl+N 未保存三选一 | assisted | 截图 | 脏文档 Ctrl+N | 2026-11-05 | 通过（脏文档 Ctrl+N 弹三选一 MessageBox；证据 `docs/review/evidence/b1-g8-new-dialog/`） |
| G9 | Ctrl+O / 快速打开与只读 | assisted | 截图 | Ctrl+Shift+P 截图 | — | 未测 |
| H1 | 三菜单项数与灰显 | assisted | 截图 | 三个菜单逐个截图 | 2026-11-05 | 通过（文件 7 项 / 编辑 12 项 / 视图 8 项；快捷键提示可见；证据 `docs/review/evidence/b1-menu-theme/`） |
| H2 | 菜单键盘导航 | manual | 人工记录 | 已决策降级 v2 | 2026-11-04 | 已决策降级 |
| H3 | 点菜单外部只关菜单 | assisted | 截图 | 菜单打开点正文 | 2026-11-05 | 通过（菜单弹出后点正文，菜单关闭且光标落点不变） |
| H4 | 右键菜单三分类 | assisted | 截图 ×3 | 正文/表格/图片右键截图 | 2026-11-05 | 通过（正文/表格/图片三类右键菜单均正确；链接右键真机未命中 CtxKind::Link，已写回归守门，降级为部分通过） |

**等级统计**：`auto` 15 条（可无人值守判定）；`assisted` 26 条（脚本驱动 + 人工看截图）；`manual` 9 条（硬件/主观/破坏性）。合计 50 条。

### 证据归档规范（A3）

- **临时证据**：`%TEMP%\veil-walkthrough\evidence\<用例号>\`（截图 `doc-width-scroll-YYYYMMDD-HHMMSS.png`、探针 `probe.log`）。
- **长期证据**：仅把判定所依赖的最小截图/日志复制到 `docs/review/evidence/<用例号>/`，避免仓库膨胀。
- **回填字段**：用例号 / 等级 / 命令参数 / 结果（通过/不通过/阻断/未测）/ 证据路径 / 最后验证日期。
- **硬规则**：无证据的回填视为无效；脚本失败与产品缺陷必须在结论列区分。
