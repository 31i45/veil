# 分层错误与恢复规约

> ADR-style 架构守护文档。本文档是 Veil 错误处理与恢复行为的权威事实来源，
> 与 `architecture-layering.md`（层级边界）、`boundary_guard.rs`（编译期门禁）互为表里。
> 目标：把原本隐式、分散的恢复行为显式化、可验证化，使"分层恢复策略"成为可审计、可回归的工程质量属性。

## 1. 核心原则

**恢复 = 错误 → 降级 → 继续**，不是 panic → 恢复。

release profile 为 `panic="abort"`：任何未捕获 panic 都是进程级终止，`catch_unwind` 在 abort 下不生效。
因此本规约的恢复能力建立在"所有**可恢复**路径都返回 `Result` 并降级"之上；
`panic` 只保留给**不可恢复的程序 bug**（内部不变量被破坏），且必须集中在已知、可审计的位置。

三条铁律：
1. **外部输入绝不应导致 panic**。文件、网络、图片、公式、mermaid、打印等一切外部输入驱动的失败，必须返回 `Result` 或产出降级结果。
2. **降级绝不静默**。任何降级/报错路径都必须保留可归因诊断（原始原因 + 位置 + 用户可执行的下一步）。
3. **不变量断言不改为兜底**。内部 `expect`/`unwrap` 是布局/光标等计算的不变量，改成 `unwrap_or_default` 会把逻辑 bug 变成不可见的错误渲染，比 panic 更难发现。

## 2. 三层错误契约

| 层 | Crate | 错误契约 | 理由 |
|---|---|---|---|
| 策略层 | `veil-core` | 纯计算、无 IO。非法内部状态属程序 bug，可 panic；对外的非法编辑返回 `EditError` | 无外部输入，错误即 bug；保持零依赖、可单测 |
| 机制层 | `veil-ui`、`veil-mermaid-offline`、`veil/src/platform` | 一切外部输入/IO/解析失败**必须返回 `Result` 并降级**，不得 panic；诊断经 `diag`/`eprintln` 归因 | 直接面对文件/网络/系统 API，失败是常态而非异常 |
| 编排层 | `veil` 顶层（`lib.rs`/`cli`/`export`） | 把机制层错误转为用户可见反馈；启动期致命错误弹框退出；运行期错误保证主循环不中断 | 唯一能与用户交互的层，负责"信任显性化" |

**层间错误流向**：策略层 `EditError` → 机制层包装为 `Result<_, String>`（i18n）→ 编排层转用户可见（模态/通知/弹框）。跨层不抛异常，只传 `Result`。

## 3. 恢复行为矩阵

> 每行均可 grep 定位。`U=crates/veil-ui/src`，`V=crates/veil/src`，`P=V/platform`，`M=crates/veil-mermaid-offline/src`。
> "诊断归因"列描述失败时原始原因如何被保留。

| # | 失败点 | 源码位置 | 失败分类 | 恢复动作 | 用户可见性 | 诊断归因 | 守护测试 |
|---|---|---|---|---|---|---|---|
| 1 | 文档加载 | `V/lib.rs:28-31,138-160` | IO / 非 UTF-8 | `OpenFailure` 归因 → 空白草稿开窗 | 首帧可重试模态 | 是：路径+原因+下一步 | `open_failure_text` 各分支 |
| 2 | 数学渲染（非法 tex） | `V/lib.rs:312-326`、`U/paint.rs` | TeX 解析/光栅 | `Err` + 记忆失败 | 源码显示 | 是：stderr | `invalid_tex_errs_for_source_fallback` |
| 3 | 数学字体初始化 | `V/lib.rs:340-362` | 字体内嵌加载 | `NoMath`/`NoExport` 恒 `Err` | 公式降级源码、导出拒绝 | 是：stderr | 见 §4 门禁 G-数学字体 |
| 4 | 本地图片加载 | `V/lib.rs:429-438` | IO / 解码 | `Err` + 记忆失败 | alt 文本 | 是：stderr | `image_loader_rejects_non_png_bytes`、`image_loader_reports_wic_failure` |
| 5 | 远程图片加载 | `U/app.rs:1550-1566` | 网络 / 解码 | 记忆失败 + 通知 | alt + 状态栏 | 是：失败原因 stderr（G1 已补） | 远程失败注入用例 |
| 6 | mermaid 渲染 | `M/lib.rs:89-100`、`U/app.rs:1590-1599` | JS / 超时 / SVG | `Err` + 三次熔断 | 源码 + 原因通知 | 是：原因裁剪进状态栏 | mermaid 失败计数用例 |
| 7 | 剪贴板初始化 | `V/lib.rs:730-743` | arboard | `Noop` 兜底 | 无（Ctrl+C/V 静默无效） | 是：stderr | 见 §4 门禁 G-剪贴板 |
| 8 | 文件保存 | `V/lib.rs:670-683` | 写 / rename | 清 tmp + `Err` + 保脏 | 模态/通知 | 是：路径+原因 | `fs_save_removes_tmp_when_rename_fails` |
| 9 | 资产迁移 | `V/lib.rs:515-555` | IO / 越界 | 逐项跳过，另存继续 | 数量通知 | 是：`report.failed` | `migrate_relative_assets_*` |
| 10 | HTTP 抓取 | `P/http.rs:373-385` | URL / HTTP / 线程 | URL 校验回传 Err；spawn 失败回传 Err（G3） | 经 #5 降级 | 是：Err 文本 | `actual_loopback_peer_fails_closed` 等 |
| 11 | IPC 单实例 | `V/lib.rs:111-133`、`P/ipc.rs` | OS / 帧 | 入口 `Err` 退出；运行期帧失败诊断后继续（G4） | 模态 | 是：入口模态；帧失败 stderr | IPC 帧/超时用例 |
| 12 | 打印 | `P/print.rs:721-748` | Win32 / GDI | 用户取消静默；API 失败诊断（G5）；版面失败回退 | 通知/模态 | 是：错误码 stderr | 打印版面回退用例 |
| 13 | HTML 导出 | `V/export.rs`、`U/document/doc_state.rs` | 媒体 / IO | 公式源码、图片 alt 或 `Err` | 模态/降级产物 | 是 | 导出失败通知用例 |
| 14 | PDF 导出 | `P/pdf_media.rs:56-69`、`P/print.rs` | 媒体 / IO | 公式经 `catch_unwind` 隔离为 Err + 媒体降级 | 模态 | 是 | `math_image_becomes_image_xobject_in_pdf` |
| 15 | 配置/主题加载 | `U/app.rs:1087-1101`、`U/theme.rs:1165` | TOML / IO | 解析错→默认+通知；读取 NotFound 静默、其余诊断（G2） | 解析错通知 | 是：解析错含原因；读取失败 stderr | 配置损坏/忽略用例 |
| 16 | 快照/会话/恢复 | `U/document/doc_state.rs:337-351` | IO / 损坏 | 写失败 `Err` 重试 + 通知；坏恢复清除 | 部分通知 | 是：stderr + 通知 | 恢复写入失败用例 |
| 17 | 文件监听 | `P/watch.rs:26-32,94-99` | notify | watch 失败 `Err` 继续编辑；错误事件诊断后跳过（G6） | 初始化通知 | 是：事件错误 stderr | `same_target` 过滤用例 |

## 4. 恢复门禁（回归测试）

为每条关键恢复路径常驻"注入失败 → 断言不 panic + 降级或可归因 Err"的测试。
已在矩阵"守护测试"列标注对应测试名。本规约要求：

- **G-数学字体**：构造数学字体不可用的 fake，断言 `NoMath.render` 与 `NoExport.export_html` 恒返回 `Err`（公式降级源码、导出拒绝），进程不 panic。
- **G-剪贴板**：构造剪贴板初始化失败的 fake，断言 `Noop` 各方法安全返回（`None`/空），不 panic。
- **G-远程图片**：注入失败结果，断言进入 `remote_failed` 记忆并保留诊断（G1）。
- **G-文档加载**：注入不存在/非 UTF-8/无权限路径，断言 `OpenFailure::{NotFound,NotUtf8,Denied}` 归因正确且 `open_failure_text` 含下一步。
- **G-配置**：注入损坏 TOML → 默认配置 + 可见通知；注入读取失败 → 默认配置 + 诊断（G2）。
- **反向验证**：人为把任一恢复路径改为 `panic!`，对应门禁测试必须失败（验证后还原）。

## 5. 错误类型层级决策

**结论：保持极简 `Result<_, String>` + i18n 模型，不引入跨模块 `From` 桥接，不引入 thiserror/anyhow。**

| 方案 | 收益 | 代价 | 裁决 |
|---|---|---|---|
| 保持 `Result<_, String>` + 3 个错误类型实现 `Display`+`Error` | 模型唯一、认知负担低、错误文本天然含 i18n 归因与下一步；可被 `format!`/`Box<dyn Error>` 消费 | 无法 `?` 自动跨模块转换；不能结构化 match 错误类型做差异化恢复 | **采纳** |
| 引入 `From` 桥接 + 结构化错误层级 | `?` 传播更简洁；可按类型差异化恢复 | 与现有 String 模型并存 = 第二套错误模型；需逐个模块迁移；差异化恢复在当前"统一降级+通知"策略下用不上 | 否决 |

理由：Veil 的恢复策略是"按失败点统一定义降级动作"（见矩阵），不依赖"按错误类型分支"。结构化层级的核心收益（类型驱动恢复）在该策略下无实际用途，引入只会制造两套并行的错误表示。3 个错误类型（`EditError`/`StartupError`/`OpenFailure`）实现 `Display`+`Error` 已满足"可被上层统一打印/盒装"的全部需求。

## 6. 已知约束与合理设计（不改代码，记录在案）

- **PDF 数学公式的 `catch_unwind` 在 release 失效**：`pdf_media.rs:64-68` 用 `catch_unwind(AssertUnwindSafe)` 把 latex-rust 内部 panic 隔离为 `Err`（dev/test 有效）。release 为 `panic="abort"`，`catch_unwind` 不生效——若 latex-rust 在极端 TeX 下 panic，release 仍会 abort。这是 spec 明确不做的"进程级 panic 恢复"；缓解依赖"latex-rust 极少 panic"与 dev 期测试覆盖。不改变 release profile。
- **watch 错误事件跳过是有意设计**：`watch.rs:94-99` 注释明确——notify 错误事件（如缓冲区溢出）不改变"磁盘上是哪一版内容"，跳过以避免一次错误吞掉同批次真实事件。G6 仅补诊断，不改变跳过语义。
- **首次运行配置文件不存在属正常**：`app.rs:1100` 对 NotFound 静默用默认配置是正确行为；G2 只把 NotFound 之外的读取失败补诊断。
- **打印"用户取消"静默**：`print.rs:721` 用户取消不打断工作流是正确行为；G5 只把 API 失败（`CommDlgExtendedError` 非 0）补诊断。

## 7. 与既有守卫的关系

- 本文档定义"错误如何恢复"，`architecture-layering.md` 定义"依赖如何分层"，两者正交。
- 恢复路径的**回归**由 §4 门禁测试守护；层间**依赖方向**由 `boundary_guard.rs` 守护。
- 新增外部输入失败点时，必须先在矩阵登记其恢复动作与守护测试，再提交实现。
