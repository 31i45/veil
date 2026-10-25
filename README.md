# Veil

Veil 是一款 Windows 桌面 Markdown 所见即所得编辑器：纯 Rust 实现，无 webview、无 Chromium/WebView2 进程、无外部运行时。

## 架构

Workspace 由三个 crate 组成，依赖方向单向（`veil` → `veil-egui` → `veil-core`，禁止反向）：

| Crate | 职责 |
|---|---|
| `veil-core` | 纯逻辑内核：文档模型、增量解析、编辑事务。零 GUI、零异步运行时依赖 |
| `veil-egui` | 视图适配器：winit + softbuffer 软件呈现 + cosmic-text 排版，chrome（菜单/状态栏/滚动条）自绘，不引入 egui/GL（ADR-005） |
| `veil` | 产品层二进制：平台接缝（Windows FFI、配置、剪贴板、文件监听）与功能装配 |

> crate 名 `veil-egui` 为历史遗留，当前并无 egui 依赖；v1 不更名，v2 再评估。

## 已实现功能

1. 文本编辑：插入/删除/撤销/重做（事务）
2. 增量解析与语义树（脏块局部重同步）
3. 光标与选区：grapheme 边界导航；光标进出 `**bold**` 等标记时展开/收起，光标视觉位置不跳
4. 文件打开/保存：自动保存、外部修改提示与覆盖前备份
5. 剪贴板：纯文本、HTML→Markdown 转换、截图粘贴落盘（`<文档名>.assets/`，递增命名不覆盖）
6. 块级渲染：表格（只读渲染）、公式（latex-rust 纯 Rust 路径）、文档内图片（相对路径 PNG，断链/远程 URL 降级为 alt 文本）
7. HTML 导出
8. 配置与主题：`[theme] mode`（system/light/dark）、跟随系统外观、深色标题栏/Mica、切换持久化
9. 自绘 chrome：菜单栏、状态栏、滚动条；系统文件对话框（打开/另存为）

## 非目标（v1 明确不做）

- wgpu/GPU 渲染视图
- 协作编辑 / CRDT
- 插件系统
- 移动端
- 表格单元格内可视化编辑（v1 仅渲染 + 源码编辑）
- Mermaid 内嵌渲染
- 任何网络功能：无同步、无更新检查、无遥测

## 构建 / 运行 / 测试

前置：Rust stable（edition 2021），Windows 11 23H2+ 为基线平台。

```powershell
cargo build --workspace     # 构建整个 workspace
cargo test --workspace      # 运行全部单元/集成测试
cargo run -p veil -- <file.md>   # 运行并打开指定 Markdown 文件
```

## 数字红线（目标值）

下表为 v1 验收红线。标「目标值」的项尚未在真机实测，在首次发布构建后逐项核对。

| 指标 | 目标值 |
|---|---|
| 冷启动（发布构建，进程创建→首帧可交互） | ≤ 300 ms |
| 10 万字文档击键→提交上屏延迟 p95 | ≤ 16 ms |
| 10 万字文档常驻内存（私有工作集） | ≤ 150 MB |
| 安装产物体积（`veil.exe` 单文件） | ≤ 15 MiB |
| `veil-core` 单元+属性测试数 | ≥ 80 |
| e2e 断言数 | ≥ 20 |
| Markdown 往返一致率（无编辑时输出 == 输入字节） | 100% |
| `veil-core` 的 GUI/异步运行时依赖数 | 0 |
| `veil-egui`+`veil` 直接依赖总数 | ≤ 8 |
| workspace 传递依赖总数 | ≤ 200 |

依赖黑名单：`tokio`、`wasmtime`、一切 webview crate（`webview2-com`/`wry` 等），引入即视为违规。

## 文档导航

- `.skills/INSTANCE.md` — 项目实例文件：全部专有事实、红线实测值、决策清单的单一真值来源
- `.skills/` — ai-dev-skills 方法论在本项目的实例化
- `docs/adr/` — 架构决策记录：
  - ADR-001 rope 选型
  - ADR-002 公式渲染路径
  - ADR-003 光标与编辑点规则
  - ADR-004 平台接缝
  - ADR-005 chrome 自绘 + 系统对话框
  - ADR-006 外部文件监听
  - ADR-007 配置与主题
  - ADR-008 依赖升级策略

## 许可

未定。
