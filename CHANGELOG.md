# Changelog

Veil 版本迭代记录。每个版本的详细修复清单见对应 git tag 的 commit log。

## [Unreleased]

### Added
- 菜单键盘导航：主菜单与右键菜单支持 ↑/↓ 选择、Enter 执行、Esc 关闭，边界钳位，复用既有悬停高亮
- 表格行列增删与列对齐：右键 6 项结构操作（上方/下方插入行、插入左列/右列、删除行、删除列；表头无行操作、单列无删除列），左/居中/右对齐只改写分隔行，一次 undo
- 单元格富文本：非活动单元格渲染粗体/斜体/删除线/行内代码/链接色并隐藏标记，活动单元格显示 Markdown 源码；窄列仍走禁则折行
  - 降级说明：极少数单元格（剥标记结果与内核 `cell.text` 不一致，如含转义 `\|`）该格降级为纯文本显示，其余单元格均为富文本；折行策略未降级
- 源码模式 `Ctrl+/`：全文显示 Markdown 源码（含表格、公式块），切换保持光标与滚动位置；打印/导出始终为渲染态
- 直接导出 PDF `Ctrl+Shift+E` / 文件菜单「导出 PDF…」：默认输出到文档同目录 `<stem>.pdf`（重名递增 ` (2)`/` (3)`），支持目录页、页码、页边距（默认 20 mm），配置写入 `config.toml`；依赖系统「Microsoft Print to PDF」，未保存草稿或无 PDF 打印机时回退系统打印对话框并提示
- 性能探针：`VEIL_PERF_PROBE=1` 输出 `[perf] {json}`（key_us/frame_us/rss_mb/dirty_blocks/total_blocks/rescan_lines/undo_depth）；`tools/walkthrough.ps1 -Perf` 汇总 p50/p95/p99、RSS、rescan 比例

### Changed
- 链接改用主题链接色（浅 `#2B6CB0` / 深 `#7FB4F5`），不再与正文同色
- `LayoutEngine::layout_block` 增加 `reveal_all`，源码模式复用同一排版路径
- `Editor::append_table_row` 更名 `replace_table_block`，表格结构变更统一走整块替换
- 发布门禁：release exe 体积断言改为 4–6 MiB；版本断言改为 `^1\.`

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
