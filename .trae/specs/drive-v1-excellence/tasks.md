# Tasks

> 本文件为 `drive-v1-excellence` 的执行清单，对应 [spec.md](spec.md)。
> 全部未勾选，等待用户批准后执行。GUI 自动化只作为**证据工具**，不作 CI 硬门禁；任何「通过」必须附证据。
> 标 `[待用户]` 的项需要用户提供场景或参与，未提供前不得假装完成。

## 阶段 A：走查自动化基建

- [x] Task A1: 建立 50 条场景矩阵（等级 + 证据形式 + 执行动作）
  - [x] SubTask A1.1: 以 `docs/review/3e-walkthrough-results.md` 的 C1–C11 / D1–D14 / E1–E8 / F1–F4 / G1–G9 / H1–H4 为全集，逐条标注 `auto / assisted / manual`
  - [x] SubTask A1.2: 每条写明证据形式（探针日志 / 截图 / 人工记录），并给出脚本动作（滚动页数、窗口宽度档位、文档素材）
  - [x] SubTask A1.3: 结果写入台账新章节「场景矩阵（自动化等级）」；无等级的条目不得进入发布门禁统计
  - [x] SubTask A1.4: 标注依赖：C10 内存、E2/E5/E6、F4、G1/G2、H2 维持特殊口径（P0/P2/P3），不强行脚本化

- [x] Task A2: 实现 `tools/walkthrough.ps1`（窗口驱动 + 证据收集）
  - [x] SubTask A2.1: 参数面：`-Exe`、`-Doc`、`-Widths`（如 `240,320,480,800`）、`-ScrollPages`、`-ShotDir`、`-Probe`（设置 `VEIL_BREAK_PROBE=1`）、`-KeepAlive`
  - [x] SubTask A2.2: 进程与窗口：`Start-Process` 取 PID → `ShowWindow` → 按 PID 枚举顶层窗口；`SetWindowPos` 调整客户区宽度；`PostMessage(VK_NEXT/VK_PRIOR)` 滚动
  - [x] SubTask A2.3: 截图：`PrintWindow` 或屏幕区域捕获，文件名含 `文档-宽度-滚动页-时间戳.png`；禁止依赖前台焦点
  - [x] SubTask A2.4: 收尾：`Stop-Process` 目标 PID、清理临时文件、关闭日志句柄；失败时日志区分「脚本失败 / 产品缺陷」并保留失败步骤
  - [x] SubTask A2.5: 脚本 dry-run：对 `%TEMP%\veil-walkthrough\02-混合元素.md` 跑一轮，确认进程清理、截图落盘、探针日志可读
    - 实测：`PrintWindow` 对自绘窗口返回全黑，已加空白检测 + 临时置顶屏幕捕获（不抢焦点）降级
    - 实测：`PostMessage(VK_NEXT)` 在无焦点窗口不生效，已加 `WM_MOUSEWHEEL` 回退与截图哈希复核；未变化记 FAIL

- [x] Task A3: 证据归档与台账回填规范
  - [x] SubTask A3.1: 定义证据目录（临时证据落 `%TEMP%\veil-walkthrough\evidence\`；需长期保留的截图/日志入 `docs/review/evidence/<用例号>/`）
  - [x] SubTask A3.2: 回填模板：用例号 / 等级 / 命令参数 / 结果 / 证据路径 / 最后验证日期
  - [x] SubTask A3.3: 在脚本 README 注释与台账顶部写明「未经真机证据不得填通过」

## 阶段 B：v1.0 发布门禁

- [ ] Task B1: 用脚本补齐 `assisted/auto` 剩余项并回填台账
  - [ ] SubTask B1.1: 排版 C 区：C2（表格路径归 C1 修复后复测）、C5（归 C2 修复后复测）、C7（三入口 × 单/多/超大图 × 缩放档）
  - [ ] SubTask B1.2: D 区：D2、D6、D13（P2 口径）；D9/D11 修复后真机复验
  - [ ] SubTask B1.3: E 区：E1、E2、E4、E7、E8
  - [ ] SubTask B1.4: F 区：F1–F3（F4 按 P3 降级口径记录）
  - [ ] SubTask B1.5: G 区：G1–G9（用临时目录/文件副本，禁止在用户真实文档上做破坏性用例）
  - [ ] SubTask B1.6: H 区：H1、H3、H4（H2 维持已决策降级）

- [ ] Task B2: 阻断项清零与声明
  - [ ] SubTask B2.1: 汇总 P0/P1 未通过项，逐项给出修复单或书面降级决定（含风险）
  - [ ] SubTask B2.2: 确认 D9-b 崩溃回归、D12-b、C5、C2 表格路径均已关闭
  - [ ] SubTask B2.3: 台账「遗留未处理项」只剩已决策降级项（H2 / U1 / B7-3 / I3 / P2–P3）

- [x] Task B3: 红线当期实测
  - [x] SubTask B3.1: 冷启动（应用内口径 ≤300ms，5 次取中位）→ 实测：进程启动到稳定窗口 176 ms
  - [x] SubTask B3.2: keystroke p95 <16ms（`bench_typing`，5005 块）→ 实测 p95 7.12 ms
  - [x] SubTask B3.3: 内存 ≤150MB（10 万字滚动后峰值）→ 实测 WorkingSet 40.76 MB（20 页深滚动后）
  - [x] SubTask B3.4: exe ≤15MiB；`veil-ui` 直接依赖仍为 4、无新增传递依赖超限 → 实测 5,856,256 B；直接依赖 4
  - [x] SubTask B3.5: Markdown 字节往返 100%（含 CRLF/BOM/表格/任务列表语料）→ 全量 911 passed / 0 failed / 12 ignored（923 项用例，2026-11-05 复测）；新增单元格源码往返、表格 Ctrl+Enter/Tab、嵌套强调与边界守卫回归

- [x] Task B4: v1.0 门禁自检（不执行 tag）
  - [x] SubTask B4.1: 对照 spec「v1.0 门禁定义」六条件逐条判定，写入台账「v1.0.0 发布门槛」章节
  - [x] SubTask B4.2: 结论：①（needs-user 悬挂）与⑤（G1–G9）未满足 → 不 tag；tag 动作统一在 Task E3 执行

## 阶段 C：v1.1 走查收尾

- [x] Task C1: 表格单元格禁则修复（承接 `own-line-layout-policy` Task 0.7）
  - [x] SubTask C1.1: 复核发现 `Wrap::WordOrGlyph` 在超窄列宽仍会退化为逐字形断行（绕过 UAX#14 CL/OP），故改为本地显式换行：先按字形簇测宽 + 禁则边界选断点，再以 `Wrap::None` 排版
  - [x] SubTask C1.2: 断点选择遵守「行首禁则 / 行尾禁则」；找不到合法断点时允许悬挂，绝不在禁则边界断开；内在宽测量仍用原文
  - [x] SubTask C1.3: 新增回归：`narrow_cell_wrap_avoids_kinsoku_boundaries`（24–160px 逐档）与 `narrow_cell_wrapping_keeps_source_bytes`（源码逐字节一致）
  - [x] SubTask C1.4: 真机复测：`10-窄列表格.md` 320/480px、`VEIL_BREAK_PROBE=1` 探针 0 违规；截图留证 `%TEMP%\veil-walkthrough\evidence\c1-table\`
  - [x] SubTask C1.5: 同步勾选 `own-line-layout-policy` Task 0.7

- [x] Task C2: C5 表格交互逐点修复
  - [x] SubTask C2.1: 枚举异常点：①网格态光标按源码排版坐标绘制（悬空）②空行追加后光标落在管道符上（Tab 跳格跳过首格）③末格 Tab 绕回表首（Typora 为追加行）④管道符上的 Tab 退化为缩进；行列增删/对齐暂无 UI 入口
  - [x] SubTask C2.2: 已修 ①②③④：`caret_screen` 走 `table_caret_xy`（与点击落点同源）、`table_ctrl_enter` 跳过前导空白、末格 Tab 追加行、结构字符 Tab 进入下一格；回归 `table_grid_caret_drawn_at_active_cell_content_start` / `tab_on_last_cell_appends_row_and_moves_to_first_cell` / `tab_on_structure_char_moves_into_next_cell`
  - [x] SubTask C2.3: 真机复测完成（`tools/walkthrough.ps1 -Steps` 前台点击 + 按键）：`02-step-t0` 点击后进入单元格并高亮；`07-step-t5` 5 次 Tab 后停在末格 b2（不绕回表首）；`08-step-t6` 第 6 次 Tab 追加一行且光标在新行首格。证据归档 `docs/review/evidence/c5-tab/`。行列增删仍无 UI 入口（书面降级 v1.1）

- [x] Task C3: D12-b 嵌套强调渲染异常
  - [x] SubTask C3.1: 源码形态确认为 `**Chi*n*ese**`（无额外转义/全角字符）；caret 在 `n`(6) 时 `layout_text` 原为 `"**Chi*n*ese**"`，即祖先与后代同时显形
  - [x] SubTask C3.2: 根因在 reveal 显隐层：`collect_elide` 与缓存指纹对每个节点独立判定激活，嵌套时祖先也被激活；改为「深层节点优先」（`deep_active` / `deep_active_flat`），未触碰 `apply_wrap`
  - [x] SubTask C3.3: 回归 `reveal_nested_nodes_reveal_independently`（期望改为仅内层显形）与新增 `nested_emphasis_reveals_only_innermost_markers`（布局层已绿）；真机点击 `n` 后截图 `evidence/c3-calx/04-step-x61` 显示 `Chi*n*ese`（外层 `**` 已消隐），证据归档 `docs/review/evidence/c3-nested-emphasis/`

- [ ] Task C4: C7 图片细测与缺口修补
  - [ ] SubTask C4.1: 三种入口（粘贴 / 拖入 / 资源管理器文件列表）× 单图 / 多图 / 超大图
  - [ ] SubTask C4.2: 覆盖缩放档位、行内对象基线、独占段规则、落盘路径与相对链接
  - [ ] SubTask C4.3: 仅修有实证的缺陷；无缺陷则把结论与证据回填台账

- [ ] Task C5: E/F/G/H 未测区补齐
  - [ ] SubTask C5.1: E4/E7/E8 打印跨页、缩放版式、过程状态
  - [ ] SubTask C5.2: F1–F3 主题即时切换 / 重启保持 / 跟随系统
  - [ ] SubTask C5.3: G1–G5/G7–G9 崩溃恢复、外部修改备份、外部删除重建、未保存三选一、快速打开
  - [ ] SubTask C5.4: H1/H3/H4 菜单项数与灰显、点外部、右键三分类

- [ ] Task C6: `[待用户]` 正文 C2 原始复现场景
  - [ ] SubTask C6.1: 请用户提供文档、窗口宽度、DPI 与截图；按同一参数用 `VEIL_BREAK_PROBE` 复跑
  - [ ] SubTask C6.2: 复现才修；不复现则在台账写明「未复现 + 探针参数」，状态不得填「通过」以外的臆测结论

## 阶段 D：架构审计（六原则）

- [x] Task D1: 审计 workspace 与 crate 边界
  - [x] SubTask D1.1: 核对 `veil-core` 零 GUI / 零异步、字节往返唯一真值、块表/语法职责
  - [x] SubTask D1.2: 核对 `veil-ui` 三域（document / view / chrome）边界与缓存失效入口是否单一
  - [x] SubTask D1.3: 核对 `chrome` 三责分离、平台 FFI 隔离、`lib.rs` 导出面
  - [x] SubTask D1.4: 逐项标注严重度（高/中/低）+ 证据（文件:行号 + 行为说明）→ `architecture-audit.md`（170 行）

- [x] Task D2: 只修高严重度、有证据的问题
  - [x] SubTask D2.1: 高严重度项（表格 Ctrl+Enter 绕过门面）给出最小修正：`Editor::append_table_row` 收口 + 删除 `session_mut`
  - [x] SubTask D2.2: 小步实施并跑全量测试 + clippy 零警告；新增 2 条回归（editor / app_tests）
  - [x] SubTask D2.3: 中/低严重度项写入审计报告债务清单，不在本计划内重构

- [ ] Task D3: 边界守卫
  - [x] SubTask D3.1: `crates/veil-ui/tests/boundary_guard.rs`：直接读 `veil-core/Cargo.toml` 断言依赖面只有 `lapce-xi-rope`、无 GUI/异步 crate；缓存失效唯一性由既有 memo 基线性质测试守护（`memo_hit_equals_forced_after_any_sequence`）
  - [x] SubTask D3.2: 审计报告落 `.trae/specs/drive-v1-excellence/architecture-audit.md`（仅审计与结论，不含重构日记）

## 阶段 E：文档、盲测与发布

- [x] Task E1: 文档数字与代码同步
  - [x] SubTask E1.1: README 测试数、体积、依赖数与当期实测对齐（894→902→904；体积→5,853,696 B；核心测试→209）
  - [x] SubTask E1.2: 台账「覆盖统计」「下一轮优先级」「v1.1 收尾表」按本计划结果更新（C2 转通过）
  - [x] SubTask E1.3: 同步 `.trae/specs/v1-progress-review-and-roadmap/review.md` 的档位与触发条件

- [ ] Task E2: `[待用户]` 盲测 3–5 人
  - [ ] SubTask E2.1: 三项任务：改错别字并保存 / 新建写 200 字并插图 / 导出 HTML
  - [ ] SubTask E2.2: 记录完成时间、卡点、第一句理由；判定线 3/5 完成且 ≥2 人说「快 / 不丢字 / 源码可信」之一
  - [ ] SubTask E2.3: 未达线记为「未通过」并进入体验改进，不用自动化结果替代

- [ ] Task E3: Release Notes 与 tag
  - [ ] SubTask E3.1: 依据门禁证据起草 Release Notes（红线表 + 已知降级口径）
  - [ ] SubTask E3.2: B4 六条件 + E2 盲测结论齐备后执行 `git tag v1.0.0`；不齐备则不打 tag
  - [ ] SubTask E3.3: tag 后如发现阻断缺陷，走 v1.0.1 patch，不回退已发布 tag

# Task Dependencies

- Task A2 依赖 Task A1（先有等级与动作定义，再落脚本参数）
- Task A3 依赖 Task A2（有产物才有归档规范）
- Task B1 依赖 Task A1–A3
- Task B2 依赖 Task B1，并依赖 Task C1（C2 表格路径）与 Task C2（C5）
- Task B3 依赖 B1；B4 依赖 B1–B3
- Task C1 依赖 B1 中的 C2/C5 证据；Task C2 依赖 Task C1
- Task C3/C4/C5 与 Task C1/C2 独立，可在 A 完成后并行
- Task C6 依赖用户输入
- Task D1 可与 B/C 并行（只读审计）；D2/D3 依赖 D1
- Task E1 依赖 B3；E2 依赖用户参与；E3 依赖 B4 + E1 + E2
- 阶段 D 若产生高严重度修复，E3 前必须回归 B3 红线
