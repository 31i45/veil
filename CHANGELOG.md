# Changelog

Veil 版本迭代记录。每个版本的详细修复清单见对应 git tag 的 commit log。

## Unreleased

「设计统一」：令牌单源、表面层级、状态过渡与便携文件关联。

### Added
- 原生 PDF 排版保真（Wave 4）：导出不再直出 Markdown 源码，改为按字形宽度换行、H1–H6 分级字号、列表/引用/Admonition/代码块/表格排版、图片与公式嵌入、页边距、页脚 `n / total`、目录页与 H1–H6 多级书签；图片/公式失败自动降级为 alt/源码，无字体时回退 GDI
- 文件关联 `veil.exe /register` / `/unregister`（F1+S9+C2）：写当前用户 ProgID、内嵌文档图标（资源 ID 2，`DefaultIcon` 用 `"<exe>",-2` 引用，不分发外置 `.ico`）、把 Veil 加入「打开方式」并只补 `.md` 空位；`/unregister` 只删自己创建且当前值仍属于自己的项。`UserChoice` 从不触碰，默认应用仍需在系统设置中选择
- 窗口几何记忆（P3）：`config.toml` 新增 `window_width` / `window_height` / `window_x` / `window_y`，启动恢复上次位置与尺寸；越界坐标夹回可见区，最大化不覆盖常规尺寸
- 状态过渡动效（S6/A9）：菜单栏、菜单项、大纲、快速打开、右键菜单、查找栏与对话框按钮的悬停/按下高亮以 120 ms ease-out cubic 淡入淡出（仅透明度）；关闭时瞬时到位
- 动效跟随系统：产品层读取 `SPI_GETCLIENTAREAANIMATION`（「显示动画」），关闭时不做过渡；仅动画进行中按帧调度重绘，空闲路径仍零定时器、不唤醒进程
- 设计令牌：`tokens::palette`（`surface_base` / `surface_card` / `surface_popup` 等表面单一入口）、`tokens::state` 交互六态、`tokens::space` / `stroke` / `motion`；`Theme::surface(SurfaceLevel)` 显式表达 Base / Card / Popup 三层

### Changed
- 浮层收敛为单线：只保留边界线，分组改用间距，不再叠加多重线条
- 查找浮栏改版：按钮与计数重排，补无命中空态与滚动条命中刻度
- 字体显式指定：UI 雅黑 13 px、正文 16 px、代码 Consolas 14 px，修复中西文混排的字体不一致
- README 徽章改为 CI 状态 + 动态 Release，并补 Platform / Portable 两枚契约徽章
- 文档口径统一：测试基线、体积、依赖数改为实测值；`.md` 关联章节改为 `/register` 流程
- 体积门禁临时统一为单一 release exe ≤ 10 MiB（10,485,760 B）；代码/图标分档暂停，Wave 5 结束后重新裁定。当前 exe 7,056,896 B（基线 6,914,048 B，+142,848 B）

### Fixed
- 代码块（fenced / indented / HTML / Front Matter）与行内代码的中文不再是空心/描边：根因是等宽路径用 generic `Family::Monospace`，摇字体时先耗尽等宽候选，本机命中旧 CJK 等宽字体 MS Gothic；改为显式等宽族名（探测到的 Consolas），Han 因此回到脚本回退的 `Microsoft YaHei UI`，与正文同族，Latin 仍为 Consolas
- 空 Mermaid 块（含纯注释块）的「点击插入模板」提示现在可点击：左键命中后在鼠标处弹出三条模板项（流程图/时序图/类图），选择后插入到该块 body 起点并把光标定位到插入内容之后，单次 undo 可整体回退

### Notes
- 历史体积口径（Phase A 基线，仅存档；当前门禁与实测见上）：发布产物曾分两档——运行时代码（非图标）6,048,424 B（门禁 ≤ 6,291,456 B = 6 MiB）、合计 exe 6,105,088 B（门禁 ≤ 6,348,120 B）；两个图标资源合计 56,664 B（`veil.ico` 32,054 + `veil-doc.ico` 24,610），是窗口/任务栏图标与 `.md` 关联必需功能，不计入代码门禁
- `cargo test --workspace` 1293 passed / 0 failed / 14 ignored；`cargo clippy --workspace --all-targets -- -D warnings` 零警告
- workspace 传递依赖 117（门禁 ≤ 200）
- 真机走查（2026-11-10）：主界面、查找栏、文件菜单（含最近文件）、未保存确认模态、视图菜单、大纲面板渲染正常，无遮挡或错位；动效时序由单元测试覆盖，未做逐帧截图取证

### 战略级推翻（2026-11-12）
- 推翻 D10⑤「HTML 白名单不做」裁决，改立项 Wave 7 N10-C 顶流行内 HTML 白名单（零脚本、零外链、纯行内 + 块级折叠）。根因：用户定调「Veil 要做顶流 Markdown 软件，所有细节必须完美」，要求对齐 Typedown / Obsidian / Notion 行为。
- 战略级推翻的处理规范：**不得静默改写已勾选项**（如 plan-remaining-backlog SubTask 2.3），必须新建推翻 SubTask（已建 2.4）与主台账 D10 子条目，保留历史 + 明确新范围 + 交叉引用。
- 实现入口：`.trae/specs/extend-n10-top-tier-html/spec.md`

### Wave 7 · 顶流行内 HTML 白名单（2026-11-12）
- 顶流锚点下实施 10 类行内 HTML：kbd/sub/sup/mark/abbr/code/a（成对）+ br/img（自闭合）+ details/summary（块级折叠）
- 三通道语义拉平：预览/打印/PDF/HTML 导出对白名单内标签一致；export 强制剥 script/style、白名单外 escape；remote img 降级占位 + alt
- 解析：InlineKind 新增 Html/HtmlSelfClosed/HtmlLiteral、BlockKind 新增 Details（默认收起、open 属性自动展开、视图态折叠）
- UI：paint 层 draw_kbd_frame 描边+圆角、draw_mark_bg 浅黄底；layout sub/sup 字号 0.75 + 基线偏移；br 硬换行；ctx_menu 新增 FoldDetails/UnfoldDetails
- 单测：veil-core 新增 10 个 inline HTML 解析测试 + 6 个 details 块解析测试；veil-ui 新增 8 个 UI 层测试；real_world_check 差异 7 篇（阈值 18），均为预期 Front Matter / Pre 折叠 / sample.md Other 块
- 体积：release exe 7,031,808 B（Wave 6 基线 6,988,800 B → +43 KB；历史基线 6,914,048 B → +117,760 B；门禁 10 MiB）

### 真机走查修复（2026-11-12，Wave 7 N10 + N9 复验）
- 行内 HTML 装饰（kbd/mark/abbr/sub-sup/code/pre）改为按下标偏移正确绘制，不再只覆盖首段，并覆盖表格单元格路径；abbr 改点线并在悬停时显示 title tooltip；`<img>` 行内改为占位/缩略图，块级单行 `<img>` 不再当源码卡
- 表格单元格内 `<br>` 正常换行；块级 `<details>` 展开正文可见、折叠行高不再裁切、卡片底色不遮字
- 行内代码中文不再多出反引号、不插空格；中置 Front Matter 不再丢行
- 导出修复：编号标题不再被当有序列表输出、行内 `<a href>` 不再丢失、交叉标签不再畸形，并补齐 kbd/mark/abbr/details/summary/img 的导出 CSS
- 体积：release exe 6,988,800 B → **7,056,896 B**（+68,096 B，门禁 10,485,760 B 内）

## v1.2.0

「手感与表达」：行级操作、可读行宽、状态可见性与焦点表达。

### Added
- 行移动 `Alt+↑` / `Alt+↓`：移动当前行，光标随行，一次 undo 单元
- 翻页 `PgUp` / `PgDn`：按视口翻页并保留目标列
- 按词删除 `Ctrl+Backspace` / `Ctrl+Delete`：复用词边界，不跨行
- 查找「区分大小写」开关（默认关、会话内保持）；`Enter` 替换当前命中并前进，`Ctrl+Enter` 全部替换
- 可读列宽：窗口宽度 >1600 px 时正文列约 40 个全角字宽并居中（≤1600 px 行为不变）
- H4–H6 分级：标记区按 2/4/6 个空格做层级缩进
- 状态栏「已保存 · HH:MM」/「未保存」：保存失败保留脏标记并可见提示
- CLI 打开失败：stderr + 可见错误框（`MessageBoxW`），以退出码 1 结束
- 光标表达焦点：聚焦实心、失焦空心，零定时器

### Changed
- 语法注释与 grid/装饰线对比度达标：注释 明 5.04:1 / 暗 4.98:1；grid 明 3.34:1 / 暗 3.55:1
- 标题段距 16 px（正文 10 px），标题与正文的层次更明显
- 菜单/导出项省略号：`导出 HTML` / `导出 PDF` 去掉多余 `…`，仅需后续对话框的项保留

### Fixed
- `Alt+↓` 行移动后画面滞后：行移动不改变块 id 与行区间，布局缓存键不含内容指纹；改为行移动后递增会话代次（回归用例 `line_move_refreshes_layout_without_further_edit`）

### Notes
- 版本号 1.2.0（workspace 版本与 exe FileVersion/ProductVersion 一致）
- release exe 6,043,136 B（门禁 ≤ 6,291,456 B，余量 248,320 B）
- `cargo test --workspace` 1132 passed / 0 failed / 13 ignored；clippy 零警告
- 击键 p95 6.781 ms（5,005 块）/ 12.379 ms（10,500 块），基线 6.677 / 12.488
- 冷启动（进程内首次编辑）290.0 ms，预热后 130.1 ms；本机实测，受系统文件缓存影响
- Windows 10 未做真机记录，不宣称已验证；Mica 不可用时降级为纯色

## v1.1.0

未单独发布，其变更并入 v1.2.0。

### Added
- 菜单键盘导航：主菜单与右键菜单支持 ↑/↓ 选择、Enter 执行、Esc 关闭，边界钳位，复用既有悬停高亮
- 表格行列增删与列对齐：右键 6 项结构操作（上方/下方插入行、插入左列/右列、删除行、删除列；表头无行操作、单列无删除列），左/居中/右对齐只改写分隔行，一次 undo
- 单元格富文本：非活动单元格渲染粗体/斜体/删除线/行内代码/链接色并隐藏标记，活动单元格显示 Markdown 源码；窄列仍走禁则折行
  - 降级说明：极少数单元格（剥标记结果与内核 `cell.text` 不一致，如含转义 `\|`）该格降级为纯文本显示，其余单元格均为富文本；折行策略未降级
- 源码模式 `Ctrl+/`：全文显示 Markdown 源码（含表格、公式块），切换保持光标与滚动位置；打印/导出始终为渲染态
- 直接导出 PDF `Ctrl+Shift+E` / 文件菜单「导出 PDF…」：默认输出到文档同目录 `<stem>.pdf`（重名递增 ` (2)`/` (3)`），支持目录页、页码、页边距（默认 20 mm），配置写入 `config.toml`；依赖系统「Microsoft Print to PDF」，未保存草稿或无 PDF 打印机时回退系统打印对话框并提示
- 性能探针：`VEIL_PERF_PROBE=1` 输出 `[perf] {json}`（key_us/frame_us/rss_mb/dirty_blocks/total_blocks/rescan_lines/undo_depth）；`tools/walkthrough.ps1 -Perf` 汇总 p50/p95/p99、RSS、rescan 比例
- HTML 导出主题化：内联 CSS 跟随当前浅/深主题（背景、正文、弱化色、链接色、代码底色、边框），深色页面含 `color-scheme: dark`；仍为单文件自包含、无外部资源
- 链接用系统默认程序打开：`Ctrl+单击`（鼠标手势，非新快捷键）http/https/mailto 链接交给系统默认浏览器/邮件客户端，光标不移动；链接右键菜单新增「在浏览器中打开」（位于「复制链接地址」之前）；非白名单协议只提示不打开，Veil 自身不发起网络请求
- GFM 脚注：`[^id]` 引用与 `[^id]: 内容` 定义；渲染态引用显示链接色 `[n]`，定义块隐藏标记并显示「编号 + 内容 + ↩」，编号按全文首次引用顺序；`Ctrl+单击` 引用跳到定义、`Ctrl+单击 ↩` 跳回首次引用；源码模式显示原始标记；HTML 导出生成 `<sup>` 引用与含返回链接的文末脚注列表
- 文档内锚点跳转：`[文本](#slug)` 的 `Ctrl+单击` 跳到对应标题；导出 HTML 的标题带 `id="<slug>"`；重复标题按 `名称`、`名称-1` 去重（与导出共用 `heading_slug` / `heading_slugs` 规则）
- 菜单分组分隔线：文件/编辑/视图三个主菜单按语义分组（新建打开 / 保存 / 导出打印 / 退出；撤销 / 剪贴板 / 格式 / 查找 / 插入），右键菜单同样分组（含表格「行 / 列 / 对齐 / 剪贴板」），最近文件块上下各一条
- 菜单项禁用态：无可撤销步时「撤销」禁点、无重做步时「重做」禁点、干净且有路径时「保存」禁点；禁用项灰显、不参与悬停与键盘导航，点击后菜单保持打开
- 草稿态导出：无关联文件时「导出 HTML / 导出 PDF」先弹保存对话框选定输出位置（取消即中止、不产生文件）；无路径草稿按 `Ctrl+S` 直接转为「另存为」
- 更一致的路径与标题：窗口标题显示文件名（不再显示完整路径）、状态栏显示 `文件名 · 父目录`、保存/另存为/恢复/导出提示统一为「动作：路径」并共用同一路径标签

### Changed
- 快捷键：`Ctrl+P` 改为快速打开（与 VS Code 一致），打印移到 `Ctrl+Shift+P`；查找栏内 `Ctrl+P` 可穿透打开浮层、`Ctrl+Shift+P` 不穿透
- 菜单文案：导出项去掉多余的省略号（`导出 HTML` / `导出 PDF`），仅真正需要后续对话框的项保留 `…`
- 链接改用主题链接色（浅 `#2B6CB0` / 深 `#7FB4F5`），不再与正文同色
- `LayoutEngine::layout_block` 增加 `reveal_all`，源码模式复用同一排版路径
- `Editor::append_table_row` 更名 `replace_table_block`，表格结构变更统一走整块替换
- 发布门禁：release exe 体积断言改为 4–6 MiB；版本断言改为 `^1\.`
- 内部端口变更：`Exporter::export_html` 增加 `ExportStyle` 与 `target` 参数（`veil` 产品层同步），调用方在主题切换点传入当前主题快照；`FileDialog` 新增带默认实现的 `save_export`（导出目标对话框）。对外 CLI / 配置不变
- 本批无新增依赖，`Cargo.toml` 未新增 crate

### Fixed
- `table_delete_row` 在仅两行正文时误删两行
- 链接与正文同色，渲染态无法辨识

### Removed
- `Style.underline`（从未渲染的死状态）

## v1.0.0 — 2026-11-05

首个稳定发布版本：单机单用户纯离线 Markdown 编辑器，零 webview、零外部运行时。

### 内核与文档模型
- 三 crate 单向依赖架构：`veil-core`（纯文本 + 增量解析）→ `veil-ui`（winit + softbuffer 自绘）→ `veil`（平台接缝）
- 增量语义树解析：脏块局部重同步，击键即重排
- 撤销/重做：事务模型，"连续性合并 + 光标移动/扩选断组"（VS Code 式）

### 编辑
- grapheme 边界光标导航（CJK 安全）
- 选区输入替换、撤销单步
- Ctrl+B 粗体 / Ctrl+I 斜体 / Ctrl+K 链接，结构感知 toggle（嵌套 `***` 正确解包裹）
- Ctrl+Shift+I 图片插入（独占段）

### 排版与中日韩细节
- 中英自动间距
- 行首/行尾禁则（正文段落 + 表格单元格）
- 满行悬挂标点
- 表格单元格内嵌网格态（Tab/Shift+Tab 导航，末格 Tab 追加行）
- 公式（latex-rust 纯 Rust 渲染）、图片相对路径、源码标记 reveal 展开/收起
- 超长单行降级提示
- 代码块语法高亮：16 种语言的关键字/字符串/注释/数字/类型着色（自研逐行状态机，不引入 syntect）

### 交互与视图
- Ctrl+滚轮缩放 / Ctrl+=Ctrl+- 档位 / Ctrl+0 还原
- F11 无边框全屏，Esc 只关浮层不退出全屏
- 专注模式 / 打字机模式（互斥切换）
- Ctrl+P 系统打印对话框（自带预览页 + Microsoft Print to PDF）
- Ctrl+Shift+P 快速打开（最近文件 + 当前目录候选）

### 写作闭环
- 中文口径字数统计（CJK 逐字 + ASCII 词）
- 任务列表复选框点击切换
- HTML 导出

### 配置与主题
- 浅色 / 深色 / 跟随系统即时切换 + 重启保持
- 深色标题栏 / Mica（Win11）
- 配置持久化到 `config.toml`

### 数据安全
- 任务管理器杀进程后自动从 recovery.dat 恢复草稿
- 外部修改 1 秒内提示 + 覆盖前备份
- 外部删除自动重建
- 无路径草稿 Ctrl+S 弹另存为，不静默丢弃
- 关窗 / Ctrl+N / Ctrl+O 未保存三选一 MessageBox

### 基线数字
- release exe：5.86 MiB（无外部 DLL 依赖）
- 冷启动：145–200 ms
- 击键 p95：7.12 ms
- 10 万字 150% DPI 滚动峰值：28.83 MB
- cargo test --workspace：910 passed / 0 failed / 12 ignored
- cargo clippy：零警告
