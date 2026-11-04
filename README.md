# Veil

Veil 是一款 Windows 桌面 Markdown 所见即所得编辑器：纯 Rust 实现，无 webview、无 Chromium/WebView2 进程、无外部运行时。

## 架构

Workspace 由三个 crate 组成，依赖方向单向（`veil` → `veil-ui` → `veil-core`，禁止反向）：

| Crate | 职责 |
|---|---|
| `veil-core` | 纯逻辑内核：文档模型、增量解析、编辑事务。零 GUI、零异步运行时依赖 |
| `veil-ui` | 视图适配器：winit + softbuffer 软件呈现 + cosmic-text 排版，chrome（菜单/状态栏/滚动条）自绘，不引入 egui/GL（ADR-005） |
| `veil` | 产品层二进制：平台接缝（Windows FFI、配置、剪贴板、文件监听）与功能装配 |

> 2026-11-03 v1 发布前将 `veil-egui` 更名为 `veil-ui`：该 crate 自 ADR-005 起已零 egui 依赖，旧名与事实不符。crate 边界与职责未变。

## 已实现功能

1. 文本编辑：插入/删除/撤销/重做（事务）
2. 增量解析与语义树（脏块局部重同步）
3. 光标与选区：grapheme 边界导航；光标进出 `**bold**` 等标记时展开/收起，光标视觉位置不跳
4. 文件打开/保存：自动保存、外部修改提示与覆盖前备份
5. 剪贴板：纯文本、HTML→Markdown 转换、截图粘贴落盘（`<文档名>.assets/`，递增命名不覆盖）
6. 块级渲染：表格（网格态 + 活动单元格内直接编辑）、公式（latex-rust 纯 Rust 路径）、文档内图片（相对路径 PNG，断链/远程 URL 降级为 alt 文本）
7. HTML 导出
8. 配置与主题：顶层 `mode` 单键（system/light/dark，兼容旧 `[theme] mode`）、跟随系统外观、深色标题栏/Mica、切换持久化
9. 自绘 chrome：菜单栏、状态栏、滚动条；系统文件对话框（打开/另存为）
10. 写作闭环：中文口径字数统计、专注模式与打字机模式（两个视图开关彼此互斥、全关即默认模式、不持久化，启动即回默认）、Ctrl+P 快速打开（最近文件 + 当前目录候选）、图片粘贴/拖拽插入
11. 表格键盘编辑：Tab / Shift+Tab 在单元格间导航（末格循环回首格），Ctrl+Enter 在表尾追加空行

## 非目标（v1 明确不做）

- wgpu/GPU 渲染视图（裁决见 ADR-011：v1 不做，仅在触发条件满足时立项）
- 协作编辑 / CRDT
- 插件系统
- 移动端
- 表格结构可视化编辑（合并/拆分单元格、列宽拖动、对齐方式 UI 切换）；内容编辑与行列键盘操作已支持
- Mermaid 内嵌渲染
- 源码编辑模式（v1 不做；以 Reveal 机制替代——光标进入块/标记即展开源码，离开收起；v2 再评估文件内切换形态）
- 任何网络功能：无同步、无更新检查、无遥测

## 构建 / 运行 / 测试

前置：Rust stable（edition 2021），Windows 11 23H2+ 为基线平台。

```powershell
cargo build --workspace     # 构建整个 workspace
cargo test --workspace      # 运行全部单元/集成测试
cargo run -p veil -- <file.md>   # 运行并打开指定 Markdown 文件
```

当前 894 项测试全绿（`cargo test --workspace` 894 passed / 0 failed / 12 ignored，2026-11-04 实测），`cargo clippy --workspace --all-targets -- -D warnings` 零警告。

## 数字红线（目标值与实测）

下表为 v1 验收红线与 2026-10-30 G2 门禁复测值（发布构建、探针串行）；击键延迟行已更新为 2026-11-03 最终复测。方法与明细见 `docs/review/G2-gate.md`。

| 指标 | 目标值 | 实测 |
|---|---|---|
| 冷启动（发布构建，`main`→首帧完成；**验收口径**） | ≤ 300 ms | 145–190 ms ✅（2026-10-30 分段实测） |
| 冷启动（墙钟，进程创建→首帧；环境观察项，不参与验收） | ≤ 300 ms | 450–529 ms（其中 440–470 ms 在 `main` 之前，属本机进程创建段开销） |
| 10 万字文档击键→提交上屏延迟 p95 | ≤ 16 ms | 3.37 ms（5.0k 块）/ 5.20 ms（10.5k 块）/ 9.25 ms（20.0k 块）✅（2026-11-03 release headless 分项探针 p95 之和，详见 `docs/review/keystroke-attribution.md`） |
| 10 万字文档常驻内存（私有工作集） | ≤ 150 MB | 21.55 MB |
| 安装产物体积（`veil.exe` 单文件） | ≤ 15 MiB | 5,840,896 B ≈ 5.57 MiB |
| `veil-core` 单元+属性测试数 | ≥ 80 | 207 |
| e2e 断言数 | ≥ 20 | 41（3 个 e2e_ 场景测试：写作闭环打开→保存→重开、表格+图片粘贴落盘、查找替换→崩溃恢复→自清）✅ |
| Markdown 往返一致率（无编辑时输出 == 输入字节） | 100% | 100% |
| `veil-core` 的 GUI/异步运行时依赖数 | 0 | 0 |
| `veil-ui`+`veil` 直接依赖总数 | ≤ 8 | 8 |
| workspace 传递依赖总数 | ≤ 200 | 141 |

## 同机对比（P2 公开基准，2026-10-30）

语料与脚本可复现：`bench/corpus/`（生成器 `tools/bench/gen_corpus.py`，其中 10MB 病态语料不入库、按需生成）、
`tools/bench/measure.ps1`。方法与口径声明见 [P2-benchmark.md](docs/review/P2-benchmark.md)。

| 指标 | Veil | MarkText | 倍率 |
|---|---|---|---|
| 常驻内存（进程树求和） | 21.51 MB | 365.01 MB | **17.0×** |
| 安装产物体积 | 5.46 MB | 488.95 MB | **89.6×** |
| 冷启动（窗口句柄口径） | 30 ms | 30 ms | 打平（该口径系统性偏袒 Electron，见报告 §3） |

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
  - ADR-011 wgpu 延迟引入（v1 不引入，触发式立项）

## 许可

未定。
