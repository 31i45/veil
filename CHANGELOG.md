# Changelog

Veil 版本迭代记录。每个版本的详细修复清单见对应 git tag 的 commit log。

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
