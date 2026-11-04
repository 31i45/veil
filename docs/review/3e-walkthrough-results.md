# 3e 真机走查结果记录

> 对照清单：[3e 真机走查清单与预期效果](3e-manual-walkthrough.md)
> 产物：`target\release\veil.exe`（2026-11-03 最终复测轮重打包，5,825,536 B ≈ 5.56 MiB）
> 测试期间不改代码；所有问题记录后统一修复。

## 环境

- OS：Microsoft Windows 11 企业版 10.0.26300
- 显示器 DPI：150%（探针实测 `GetDpiForWindow` = 144）
- 输入法：中文拼音 + 英文键盘布局
- 安全软件：待填

## 交接（2026-11-02 集中修复轮）

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

| # | 项目 | 实测 | 判定 |
|---|---|---|---|
| A2 | 冷启动墙钟（5 次，no args） | 201 / 78 / 77 / 80 / 77 ms，中位 **78ms** | ✅ ≤300ms（首次 201ms 为磁盘冷读） |
| A5 | exe 文件属性 | FileVersion=ProductVersion=0.1.0；描述「Veil Markdown 编辑器」；`ExtractAssociatedIcon` 取回 32×32 图标；体积 5,815,808B≈5.55MiB | ✅ |
| A1（机检部分） | PE subsystem | =2（GUI，无控制台窗口） | ✅ |

## 人工项（用户观察后逐条回填）

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

以下 C/D/E/F/G/H 共 50 条**均为 `needs-user`**（H2 已决策降级，见该表行）。状态列不写「通过」：`C 自动化覆盖（测试名）` 只表示存在对应单元/集成用例，**不等于真机验收通过**；未列测试名的条目纯人工。I3 根因已定位（2026-11-04）：编辑路径有效（测试 `toggle_wrap_inside_table_cell_edits_source`——cell 内 ToggleWrap 正确修改源码、单步可撤销）；「无效」为渲染层错觉——`shape_cell` 纯文本排版、单元格不解析行内样式，标记以字面文本显示。处置：v1 口径固化（单元格显示源码文本），富文本单元格渲染留 v2 立项。

### C. 排版与中日韩细节（11）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| C1 | 中英混排自动间距 | `needs-user` | C 自动化覆盖（cjk_to_ascii_adds_space / ascii_to_cjk_adds_space / both_boundaries_add_spaces） |
| C2 | 行首/行尾禁则 | `needs-user` | C 自动化覆盖（kinsoku_forbidden_chars_never_start_a_visual_line / kinsoku_open_brackets_never_end_a_visual_line） |
| C3 | 满行悬挂标点 | `needs-user` | C 自动化覆盖（hanging_punctuation_shifts_full_line_end_glyph / kinsoku_glue_keeps_ellipsis_attached_to_previous_char） |
| C4 | 标题/粗体/斜体/删除线/列表/引用/分隔线/代码块层级与行高稳定 | `needs-user` | C 自动化覆盖（heading_layout_taller_than_paragraph / reveal_hides_strong_markers_when_caret_outside，仅布局与显隐，未覆盖目检） |
| C5 | 表格点入编辑、Tab 跳格、行列增删、对齐渲染 | `needs-user` | C 自动化覆盖（table_renders_grid_when_caret_outside / table_shows_grid_when_caret_inside_cell / alignment_offsets_glyphs_within_cell / tab_advances_to_next_cell_start） |
| C6 | 任务列表复选框点击切换 | `needs-user` | C 自动化覆盖（task_checkbox_click_toggles_and_keeps_caret） |
| C7 | 图片粘贴/拖入/资源管理器文件列表粘贴 | `needs-user` | C 自动化覆盖（paste_image_saves_asset_and_inserts_relative_link / paste_image_files_prefer_file_list_over_path_text / paste_image_requires_document_path） |
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
| D12 | `**粗体**` 内 Ctrl+I 不误拆 | `needs-user` | C 自动化覆盖（italic_on_bold_stacks_instead_of_unwrapping / italic_on_cjk_bold_stacks_instead_of_unwrapping） |
| D13 | 视图菜单缩放/全屏与快捷键提示 | `needs-user` | C 自动化覆盖（zoom_menu_and_key_paths / menu_definitions_cover_shortcuts） |
| D14 | 纯视图操作不改文档、不置脏 | `needs-user` | C 自动化覆盖（zoom_is_view_only_keeps_doc_and_dirty / fullscreen_headless_noop_and_menu_path） |

### E. 打印 / 导出 PDF（8）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| E1 | 文件「打印 / 导出 PDF…」弹系统打印对话框 | `needs-user` | C 自动化覆盖（ctrl_p_prints_and_ctrl_shift_p_quick_opens，仅验证打印端口调用，不覆盖系统对话框） |
| E2 | Microsoft Print to PDF 保存成功 | `needs-user` | —（系统 PDF 对话框走平台 FFI，无单测） |
| E3 | 多页内容连续、无重字/截字、页边距均匀 | `needs-user` | C 自动化覆盖（print_paginates_clean_frames_and_restores_state / print_breaks_inside_tall_block_at_line_tops，仅分页断点） |
| E4 | 公式/表格跨页硬切、不重叠/不漏 | `needs-user` | C 自动化覆盖（print_hard_cuts_atomic_block_taller_than_page，仅超高原子块硬切） |
| E5 | 页码范围打印 | `needs-user` | — |
| E6 | 物理打印机抽验 | `needs-user` | — |
| E7 | 放大到 150% 后打印版式不变 | `needs-user` | C 自动化覆盖（zoom_ignored_during_print） |
| E8 | 打印过程无闪烁/残留遮罩，结束回编辑态 | `needs-user` | C 自动化覆盖（print_paginates_clean_frames_and_restores_state，仅状态复原） |

### F. 主题与设置（4）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| F1 | 浅色/深色/跟随系统即时切换与「当前」标记 | `needs-user` | C 自动化覆盖（switching_theme_persists_config / theme_menu_marks_current_mode） |
| F2 | 深色重启保持、config.toml 一致 | `needs-user` | C 自动化覆盖（startup_applies_configured_mode / switching_theme_persists_config） |
| F3 | 跟随系统时系统切深浅即时生效 | `needs-user` | C 自动化覆盖（system_mode_follows_live_switch_and_clears_color_caches） |
| F4 | Win11 Mica / Win10 不崩 | `needs-user` | —（DWM/Mica 为平台实现，无单测） |

### G. 数据安全（9）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| G1 | 任务管理器杀进程后重启恢复内容 | `needs-user` | —（进程级场景，恢复读写单测不覆盖） |
| G2 | 恢复后 Ctrl+S 保存并清 recovery.dat | `needs-user` | — |
| G3 | 外部修改 1s 内提示并挂起自动保存 | `needs-user` | C 自动化覆盖（external_change_notifies_holds_autosave_and_keeps_document） |
| G4 | Ctrl+S 先备份外部版本再覆盖 | `needs-user` | C 自动化覆盖（manual_save_conflict_backs_up_then_overwrites / failed_backup_aborts_save_without_overwrite） |
| G5 | 外部删除后提示并重建文件 | `needs-user` | C 自动化覆盖（external_delete_notifies_and_autosave_recreates） |
| G6 | 无路径草稿 Ctrl+S 提示另存为、不静默丢弃 | `needs-user` | C 自动化覆盖（save_draft_without_path_is_noop，仅验证不写盘，未断言提示文案） |
| G7 | 关窗/退出三选一、清快照 | `needs-user` | C 自动化覆盖（close_confirmation_saves_discards_or_cancels / clean_close_skips_confirmation_and_clears_stale_snapshot） |
| G8 | Ctrl+N 未保存三选一 | `needs-user` | C 自动化覆盖（new_draft_refuses_when_dirty / untitled_save_choice_cancelled_aborts_new_draft / untitled_save_choice_saves_as_then_creates_new_draft） |
| G9 | Ctrl+O / Ctrl+Shift+P 快速打开与只读 | `needs-user` | C 自动化覆盖（quick_open_lists_recent_and_directory_then_opens / quick_open_refuses_dirty_document_and_esc_closes） |

### H. 菜单与键盘可达性（4）

| # | 项目 | 状态 | 自动化覆盖 |
|---|---|---|---|
| H1 | 三个菜单项数、灰显状态 | `needs-user` | C 自动化覆盖（menu_definitions_cover_shortcuts / paint_draws_menu_status_and_popup） |
| H2 | 菜单打开后 ↑↓ / Enter / Esc 键盘操作 | ✅ 已决策（降级，v1 不承诺键盘导航） | —（无菜单键盘导航自动化用例） |
| H3 | 点击菜单外部只关菜单、不动光标 | `needs-user` | C 自动化覆盖（click_outside_open_menu_closes_it_without_moving_caret） |
| H4 | 右键菜单三分类正确 | `needs-user` | C 自动化覆盖（items_cover_three_contexts / layout_clamps_into_viewport / hit_round_trips_items，部分：分类集合+命中，未覆盖真实右键目标判定） |

**覆盖统计**：50 条中 42 条有至少一个对应自动化用例（含部分覆盖），8 条无自动化用例（C10、E2、E5、E6、F4、G1、G2、H2）；其中 H2 已于 2026-11-04 决策降级（v1 承诺 Esc 关闭 + 快捷键直达，导航留 v2），不再是待真机验证项，其余 7 条纯人工。

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
