# WYSIWYG 复杂内容收口证据（2026-11-27，Task 4）

覆盖：4.1 窄窗宽表可读完（F-010）、4.3 表格/图片/公式/图表降级四态复核、4.4 非目标引用。
4.2 Mermaid 深色逐图证据见 [`mermaid-dark-2026-11-27.md`](mermaid-dark-2026-11-27.md)。

## 1. 4.1 窄窗宽表可读完（方案 A：横向平移）

### 1.1 事实修正与方案选择

审计 F-010 引用的 `table_strategy.rs` 顶部注释（「触底仍超宽时向右溢出并被 frame 裁剪」）已过时：
当前默认路径还有第二次收缩（绝对下限 `0.4 × MIN_COL_WIDTH`），表格默认不会横向溢出版心。
但由此产生的默认结果是「多列被压到约 16px」，虽然没有裁切，阅读体验接近逐字竖排。
此外既有测试 `narrow_twelve_column_table_fits_and_tail_does_not_overlap`
（420px + 12 列）明确要求默认网格必须收纳进版心，不能删除或放宽。

因此采用**显式可读宽表模式 + 横向平移**（方案 A）：

- 指针位于表格上时 `Shift+滚轮` → 该表进入「可读宽表」：列宽不再压回版心（每列仍不低于
  `MIN_COL_WIDTH`），表宽可大于可用宽；平移量 `pan` 夹在 `[0, max(0, 表宽 - 可用宽)]`。
- 若该表内在宽本来就不超版心，则本次 `Shift+滚轮` 不接管，继续滚动正文。
- 默认（未进入可读模式）完全保留原行为，既有 12 列收纳断言不变。
- 打印/PDF 强制回到默认收纳模式（`table_is_wide` 在打印态恒 `false`），不改变 Task 3 的打印分页。
- 溢出表格底部绘制 2px 极简横向指示（轨道=可用宽、滑块=可见宽/表宽），仅作提示，不是拖拽控件。

### 1.2 同一坐标口径

`TableLayout.pan` 是唯一真值，以下四处共用：

| 路径 | 位置 | 处理 |
|---|---|---|
| 绘制网格 | `view/table_view.rs::paint_table` | `x0 = content_x + layout.x - layout.pan` |
| 光标矩形 | `paint_table.rs::table_caret_xy` | 同式再叠加单元格 `cell.x` |
| 命中（点击/选择/上下移动） | `view/table_strategy.rs::TableLayout::hit` | 先把内容坐标换回网格坐标：`x + pan - self.x` |
| 列宽拖动 | `interaction.rs::try_start_col_drag` / `drag_col_to` | `x + pan - indent` |

状态按 `BlockId` 存放在 `TableView.pan`（`app.rs`），文档切换/整篇替换时清空
（`document/doc_state.rs` 三处）。`TableKey` 增加 `wide` 位，宽/窄模式切换时重建网格；
`pan` 变化只增量修改缓存中的 `TableLayout.pan` 与视图态，不触发重排。

### 1.3 验收测试

`crates/veil-ui/src/app_tests/tables.rs`：
**`narrow_window_wide_table_readable_via_shift_wheel_pan`**

- 520px 窗口 + 8 列宽表；默认断言 `TableLayout.width ≤ content_width`（收纳）；
- `Shift+滚轮` 后断言 `TableLayout.width > content_width`，且 `pan == 表宽 - 可用宽`；
- 最右列绘制矩形完整落在版心内（左缘 ≥ 0，右缘 ≤ 版心右缘）；
- 点击最右列内容起点，`hit_offset()` 与底层 `hit_block()` 均落在该单元格的 doc_start；
- 光标设到该单元格后，`active == Some(15)`，光标矩形 x 等于「内容左缘 - pan + cell.x」，且落在版心内。

### 1.4 已知边界

- 入口为 `Shift+滚轮`（与审计建议一致），表格底部指示条提供可见性提示；不提供独立可拖拽滚动条，
  已登记于 `docs/review/deferred-and-manual-register-2026-11-27.md` §2「宽表横向滚动的表内独立滚动条增强」。
- 平移为纯视图态，不写回 Markdown、不进撤销栈；换文档即归零。

## 2. 4.3 降级四态复核（表格 / 图片 / 公式 / 图表）

| 内容 | 失败 | 空内容 | 加载中 | 远程被阻止 | 结论 |
|---|---|---|---|---|---|
| 表格 | 不适用（同步解析）；语法不成立时不成网格，按普通段落继续编辑 | 表头 + 0 行表体：仅渲染表头网格，无额外提示（无需操作入口） | 不适用（同步排版） | 不适用（无网络） | 一致，无缺口 |
| 图片 | 本地读取失败 → `[alt]` 占位 + 浅灰占位框；右键首项「图片文件缺失（当前显示替代文本）」、复制路径、重新加载；状态栏 `图片缺失` 提示 | `src` 为空/不可解析 → `[alt]` 或 `[图片]` 占位，可正常编辑源码 | 远程图首次出现 → 先显示 alt 占位，回包成功后替换；无独立 loading 文案/骨架 | 默认阻止 → alt 占位；全局入口「视图 → 允许远程图片」+ 设置项 + 切换状态栏提示 | 本次补一个小缺口（见下） |
| 公式 | 渲染失败且非编辑态 → 显示源码 + 块级提示「公式渲染失败，已显示源码」；点入即编辑源码 | 空公式按渲染失败处理（`公式渲染为空`）→ 源码 + 同一提示 | 不适用（同步渲染） | 不适用（本地数学字体/渲染器，无网络） | 一致，无缺口 |
| 图表（Mermaid 各图型） | 保留围栏源码 + `◈ 类型` 徽标；状态栏「图表渲染失败：{原因}；下一步：检查语法后重新预览」；同一源码连续失败 3 次停止自动重试；右键：mermaid.live / 预览源码图 / 复制源码 / 插入快照 | 空块提示「空 Mermaid 块 — 点击插入模板」，右键直接给流程图/时序图/类图模板 | 异步渲染期间显示源码与徽标，成功后替换；无独立 spinner | 离线渲染（QuickJS + 本地 mermaid.js），不发起网络请求；mermaid.live 仅复制源码，打开失败有明确指引 | 一致，无缺口；loading 无进度提示为已知轻微限制 |

### 2.1 本次最小修复（远程图片被阻止时的可操作入口）

问题：远程图片被全局阻止时，「重新加载图片」清缓存/重排后仍被阻止，界面毫无反馈，
用户会以为是坏图而反复点击。

修复：`interaction_menu.rs` 的 `CtxAction::ReloadImage` 在「远程 URL 且
`allow_remote_images == false`」时不再静默重排，改为状态栏提示新文案
`NoticeRemoteImageBlockedRetry`（中/英）：「已阻止远程图片；下一步：在「视图 → 允许远程图片」
（或设置）中开启后重试」；不擅自改变配置。

测试：`reload_blocked_remote_image_points_to_allow_remote_images`（`app_tests/media.rs`）。

仍未做（登记为后续）：右键菜单中的「就地允许远程图片」动作需要扩展 `CtxKind::Image`
与上下文动作表，改动面大于本轮「最小修复」，保留全局菜单/设置入口。

## 3. 4.4 非目标登记（引用）

以下不在本轮实现，登记见
`docs/review/deferred-and-manual-register-2026-11-27.md` §2：

- 表格合并 / 拆分单元格；
- 独立富文本文档模型；
- 图表渲染内置浏览器 / WebView 引擎；
- 宽表「表内独立滚动条」交互增强（本轮只做可读完的平移 + 指示条）。

## 4. 门禁结果（本机实测）

| 命令 | 结果 |
|---|---|
| `cargo test -p veil-ui` | 1471 passed, 0 failed, 12 ignored；`boundary_guard` 7 passed |
| `cargo test -p veil-mermaid-offline` | 27 passed, 0 failed, 2 ignored（另两组子进程用例各 1 passed） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo fmt --all --check` | 通过 |

全量 workspace 测试按计划由 Task 7 统一执行。

## 5. 改动文件

- `crates/veil-ui/src/view/table_strategy.rs`（`wide` 列宽策略、`TableLayout.pan`、命中坐标）
- `crates/veil-ui/src/view/table_view.rs`（绘制平移、底部指示）
- `crates/veil-ui/src/paint_table.rs`（宽表排版与 pan 夹取、`Shift+滚轮` 入口）
- `crates/veil-ui/src/view/cache.rs`（`TableKey.wide`）
- `crates/veil-ui/src/app.rs`（`TableView.pan`）
- `crates/veil-ui/src/document/doc_state.rs`（换文档清空）
- `crates/veil-ui/src/interaction.rs`（滚轮接管、列宽拖动坐标）
- `crates/veil-ui/src/interaction_menu.rs`（远程图片阻止重载提示）
- `crates/veil-ui/src/i18n.rs`（新增提示文案中/英）
- `crates/veil-ui/src/app_tests/tables.rs`（4.1 验收测试）
- `crates/veil-ui/src/app_tests/media.rs`（4.3 修复测试）
- `crates/veil-mermaid-offline/src/lib.rs`（深色 corpus 证据、journey/c4 覆盖、新增自动断言）
- `docs/qa/evidence/mermaid-dark-2026-11-27/`（22 图深色 SVG/PNG + 浅色对照 + index）
