# 异常四态矩阵（空 / 加载 / 错误 / 降级）

- 版本：v1（2026-11-27）
- 状态：生效；与 `docs/design/visual-contract.md` 配套，冲突时以视觉契约为准
- 适用：屏幕渲染（`crates/veil-ui`）与导出（`crates/veil`）
- 语义铁律：**空态不是错误。** `danger` 只用于危险操作（删除/清空）与真错误（文件缺失、导出失败、非法正则、公式/图表渲染失败）；“无结果/空列表”一律取中性色（`theme.muted`），入口为 `tokens::state::empty_fg`。
- 颜色语义列取值：中性（`muted`/`text`）、次要（info notice）、危险（`danger`）。

## 一、矩阵（11 个视图/场景 × 4 态 = 44 格）

| 视图/场景 | 状态 | 文案 key（或“无”） | 颜色语义 | 可操作（按钮或键位） | 是否持久 | 现有实现位置（文件:函数） |
|---|---|---|---|---|---|---|
| 正文文档区 | 空 | `doc.empty_hint`（空文档首行引导） | 中性（`muted`） | 直接输入；`Ctrl+O` 打开已有文件 | 否（提示不落盘；正文内容随保存写入） | `crates/veil-ui/src/paint.rs::paint_empty_hint`（`render` 调用） |
| 正文文档区 | 加载 | 不适用（打开/保存为同步 I/O，无分片加载态） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/document/doc_state.rs::open_path_now` |
| 正文文档区 | 错误 | `modal.open_failed.title/body`、`modal.save_failed.*`、`modal.save_as_failed.*`（按操作区分） | 危险（错误模态标题/徽标） | 模态按钮：重试（R）/ 重新选择 / 取消（Esc） | 否（失败不写目标文件；修改仍留在编辑器） | `crates/veil-ui/src/document/doc_state.rs::show_open_error`、`show_error_modal` |
| 正文文档区 | 降级 | 图片失败/远程被阻止 → alt 文本；公式失败 → TeX 源码 + 块右上角 `math.render_failed_hint`；`Ctrl+/` 源码模式 | 中性（alt/源码按正文色）+ 危险（公式失败提示） | `Ctrl+/` 切源码模式；点击图片可重新加载 | 否（源码模式为会话态，不写配置） | `crates/veil-ui/src/paint.rs::ensure_layouts_range`、`ensure_math_range`、`math_failure_notice`；`crates/veil-ui/src/view/layout.rs::fenced_subs` |
| 查找/替换 | 空 | 无命中计数 `i/n`（`empty_fg`）；空查询占位 `search.placeholder` / `search.replace_placeholder` | 中性（`muted`；无命中时 ‹/› 用 `disabled_fg` 置灰） | 继续输入；Esc 关闭；无命中时翻页按钮禁用 | 否（查询串与开关为会话态） | `crates/veil-ui/src/view/search_view.rs::paint_search` |
| 查找/替换 | 加载 | 不适用（每次输入同步扫描，无异步加载） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/view/search_strategy.rs::find_all` / `find_all_regex` |
| 查找/替换 | 错误 | `search.regex_error`（计数区）+ 查询串危险色 | 危险（`theme.danger`） | 点击 `.*` 关闭正则，或修正模式；Esc 关闭 | 否（`regex`/`case_sensitive` 为会话态） | `crates/veil-ui/src/view/search_view.rs::paint_search` |
| 查找/替换 | 降级 | 不适用（扫描只有“非法正则”一种失败，已归入错误态） | 不适用 | 不适用 | 不适用 | — |
| 快速打开 | 空 | `quick_open.empty.no_candidates` / `quick_open.empty.no_match` / `command_palette.empty` | 中性（`muted`） | 继续输入；Esc 关闭 | 否（候选为临时态） | `crates/veil-ui/src/paint_overlay.rs::paint_quick` + `crates/veil-ui/src/view/quick_open.rs::empty_message_for` |
| 快速打开 | 加载 | 不适用（打开时同步列目录，无索引/异步加载态） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/interaction.rs::refresh_quick` |
| 快速打开 | 错误 | `quick_open.read_failed`（目录列表读取失败） | 危险（`theme.danger`） | Esc 关闭后重新打开（`Ctrl+P`）重试 | 否（失败态随浮层关闭清空） | `crates/veil-ui/src/interaction.rs::refresh_quick` + `crates/veil-ui/src/paint_overlay.rs::paint_quick` |
| 快速打开 | 降级 | 不适用（无降级通道；草稿无目录时只列最近文件属正常子集） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/interaction.rs::refresh_quick` |
| 大纲 | 空 | `outline.empty` | 中性（`muted`） | `Ctrl+Shift+O` 关闭；Esc | 否 | `crates/veil-ui/src/paint_overlay.rs::paint_outline` |
| 大纲 | 加载 | 不适用（条目从已解析块派生，无独立 I/O） | 不适用 | 不适用 | 不适用 | 文档索引（`document` 解析产物） |
| 大纲 | 错误 | 不适用（无 I/O，纯派生不会失败；解析异常由正文区错误态承接） | 不适用 | 不适用 | 不适用 | — |
| 大纲 | 降级 | 不适用（无降级路径） | 不适用 | 不适用 | 不适用 | — |
| 版本历史 | 空 | `history.empty` + `history.empty_hint` | 中性（正文色 + `muted`） | Esc / 底部“关闭”按钮 | 否（只读面板） | `crates/veil-ui/src/paint_overlay.rs::paint_history` |
| 版本历史 | 加载 | 不适用（`open_history` 同步读取快照清单；大目录可能短暂阻塞，无进度态） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/document/snapshot.rs::open_history` |
| 版本历史 | 错误 | `history.error.headline` + `history.error.hint` | 危险（`theme.danger`） | 关闭面板后重试（Esc 关闭再 `Ctrl+Shift+H`） | 否 | `crates/veil-ui/src/paint_overlay.rs::paint_history`（错误源 `document/snapshot.rs::open_history`） |
| 版本历史 | 降级 | 不适用（只读面板，无自动降级） | 不适用 | 不适用 | 不适用 | — |
| 图片预览 | 空 | 不适用（无 `image_preview` 时不绘制遮罩） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/view/image_preview.rs::paint` |
| 图片预览 | 加载 | 无（仅压暗遮罩；位图未就绪时不画标题/提示） | 中性 | Esc / 点击遮罩关闭 | 否 | `crates/veil-ui/src/view/image_preview.rs::paint` |
| 图片预览 | 错误 | `image.preview_unavailable`（位图缺失） | 中性（`tokens::state::empty_fg`） | Esc / 点击遮罩关闭 | 否 | `crates/veil-ui/src/view/image_preview.rs::paint` |
| 图片预览 | 降级 | 不适用（`zoom ≤ 0` 视为适应窗口，属正常路径） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/view/image_preview.rs::image_rect` |
| 设置弹窗 | 空 | `settings.font_no_match`（字体候选弹层无匹配） | 中性（`muted`） | 继续输入；Esc 关闭候选弹层 | 否（候选为临时态） | `crates/veil-ui/src/view/settings.rs::paint_font_picker` |
| 设置弹窗 | 加载 | 不适用（字体列表与字段值均本地同步构造） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/view/settings.rs::paint` |
| 设置弹窗 | 错误 | `notice.config_not_saved`（保存配置失败）；非法数值提交时夹取，无独立错误态 | 危险（错误 notice） | 修正后再次“确定”保存 | 否（失败未写配置文件） | `crates/veil-ui/src/app.rs::save_config` |
| 设置弹窗 | 降级 | `notice.config_ignored`（未知键）/ `notice.config_invalid`（解析失败 → 默认配置） | 次要（info notice） | 手动修正配置文件后重启 | 是（问题源在用户配置文件，未自动改写） | `crates/veil-ui/src/app.rs::App::new`（`config.load` 分支） |
| 导出（HTML / PDF / 打印） | 空 | `notice.output_empty_document`（空文档守卫，菜单项同时禁用） | 危险（错误 notice） | 先输入内容；空文档时菜单/快捷键 no-op | 否（不产生文件） | `crates/veil-ui/src/interaction_menu.rs::output_allowed` |
| 导出（HTML / PDF / 打印） | 加载 | 不适用（HTML 同步生成；PDF/打印同步分页排版，无进度条，大文档会阻塞 UI） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/document/doc_state.rs::export_now`；`crates/veil/src/platform/print.rs::build_print_layout` |
| 导出（HTML / PDF / 打印） | 错误 | HTML：`modal.export_failed.*` + `notice.export_failed`；PDF：`modal.export_pdf_failed.*`；打印：`modal.print_failed.*` + `print.err.*`；取消：`notice.export_cancelled` | 危险（错误模态 + 错误 notice） | 弹窗“重试（R）/ 取消（Esc）”；取消导出为正常提示 | 否（失败不写目标文件；临时文件会清理） | `crates/veil-ui/src/document/doc_state.rs::export_now`；`crates/veil/src/platform/print.rs::export_pdf_error`、`print_error` |
| 导出（HTML / PDF / 打印） | 降级 | HTML：公式失败 → `<pre class="math-source">`；缺失 Mermaid 预览 → `export.mermaid_unavailable` 注释 + 源码块。PDF/打印：图片/公式失败 → alt/源码文本；Mermaid 无数据 → 源码代码块 | 危险（HTML 公式源码）/ 中性（PDF 文本降级、Mermaid 源码） | 导出前先预览公式与 Mermaid；失败可修正后重新导出 | 是（降级结果写入导出文件） | `crates/veil/src/export.rs`（`math-source` 样式、`block` 的 Mermaid 分支）；`crates/veil/src/platform/pdf_layout.rs::mermaid_block` |
| 远程图片（正文内） | 空 | 不适用（文档不含远程图片时不产生任何占位） | 不适用 | 不适用 | 不适用 | —（`crates/veil-ui/src/paint.rs::ensure_layouts_range`） |
| 远程图片（正文内） | 加载 | 无（首次遇见图发起请求，未就绪按 alt 文本占位） | 中性（alt 用正文色） | 等待；被阻止时可在设置开启“允许远程图片” | 否（待处理/缓存只在内存） | `crates/veil-ui/src/paint.rs::ensure_layouts_range`、`request_remote_image`；`crates/veil-ui/src/app.rs::poll_remote_images` |
| 远程图片（正文内） | 错误 | `notice.remote_image_failed`（状态栏错误提示），正文降级为 alt | 危险（错误 notice）+ 中性 alt | 检查 URL/网络；或在设置中允许远程图片后重试 | 否（`remote_failed` 为内存缓存） | `crates/veil-ui/src/paint.rs::ensure_layouts_range`；`crates/veil-ui/src/app.rs::poll_remote_images` |
| 远程图片（正文内） | 降级 | 关闭“允许远程图片” → alt 文本；切换时 `notice.remote_images_blocked` / `notice.remote_images_allowed` | 次要（info notice）+ 中性 alt | 设置或菜单“允许远程图片”（写入配置） | 是（`allow_remote_images` 持久化到配置文件） | `crates/veil-ui/src/app.rs::toggle_remote_images`；`crates/veil-ui/src/paint.rs::ensure_layouts_range` |
| Mermaid（正文内） | 空 | `mermaid.empty_hint`（“空 Mermaid 块 — 点击插入模板”） | 中性（正文色） | 点击/光标进入后经右键菜单插入模板 | 是（插入模板会改文档） | `crates/veil-ui/src/view/layout.rs::fenced_subs` |
| Mermaid（正文内） | 加载 | 无（自动渲染请求进行中，块暂按源码显示） | 中性 | 等待；可右键预览当前源码图 | 否 | `crates/veil-ui/src/app.rs::trigger_mermaid_auto` |
| Mermaid（正文内） | 错误 | `notice.mermaid_render_failed`（含原因） | 危险（错误 notice） | 修改源码后自动重试；右键“在 mermaid.live 打开”/预览 | 否（连续失败计数在内存） | `crates/veil-ui/src/app.rs::poll_remote_images`（Mermaid 结果分支） |
| Mermaid（正文内） | 降级 | `notice.mermaid_source_too_large`（源码 >128 KiB 跳过渲染，保留源码） | 危险（错误 notice） | 精简源码后自动重试 | 否 | `crates/veil-ui/src/app.rs::trigger_mermaid_auto` |
| 公式（正文内） | 空 | 不适用（空公式无内容，按源码空块显示，无专门空态文案） | 不适用 | 不适用 | 不适用 | — |
| 公式（正文内） | 加载 | 不适用（`ensure_math_range` 同步渲染；未就绪即显示源码） | 不适用 | 不适用 | 不适用 | `crates/veil-ui/src/paint.rs::ensure_math_range` |
| 公式（正文内） | 错误 | `math.render_failed_hint`（块右上角小字，不改布局） | 危险（`theme.danger`） | 改源码或删除公式后自动重试 | 否（`math_failed` 为内存缓存） | `crates/veil-ui/src/paint.rs::ensure_math_range` + `math_failure_notice` |
| 公式（正文内） | 降级 | 无（TeX 源码即降级形态）；光标进入公式块或源码模式时显示源码 | 中性 | 移动光标出块，或 `Ctrl+/` 切源码模式 | 否（源码模式为会话态） | `crates/veil-ui/src/paint.rs::math_shows_image`、`render` |

## 二、已实现（本轮关闭的缺口）

| 编号 | 原缺口 | 修复 | 位置 | 测试 |
|---|---|---|---|---|
| G1 | 公式渲染失败只写 stderr 并降级源码，无界面提示 | 失败块右上角绘制一行 `math.render_failed_hint`（`theme.danger` 小字），不新增弹窗、不改布局高度；光标进入块内按源码编辑时不叠加提示 | `crates/veil-ui/src/paint.rs::math_failure_notice`（渲染循环 `BlockKind::Math` 分支） | `app_tests/media.rs::math_render_failure_degrades_to_source` |
| G2 | 快速打开的目录读取失败被 `.ok()` 吞掉，与“无候选”同貌 | `refresh_quick` 记录 `read_failed`；浮层空结果改用 `quick_open.read_failed`（危险色），Esc 关闭后重新打开即重试 | `crates/veil-ui/src/interaction.rs::refresh_quick`、`paint_overlay.rs::paint_quick`、`view/quick_open.rs::empty_notice` | `view/quick_open.rs::read_failure_is_distinct_from_empty_and_marked_as_error`、`app_tests/persist.rs::quick_open_surfaces_directory_read_failure` |
| G3 | 图片预览位图缺失时静默只剩遮罩 | 保留遮罩并居中绘制 `image.preview_unavailable`，取 `tokens::state::empty_fg` 中性色（缺失非错误操作，不用危险色） | `crates/veil-ui/src/view/image_preview.rs::paint` | `view/image_preview.rs::missing_bitmap_hint_is_neutral_and_localized`、`app_tests/app.rs::image_preview_missing_bitmap_draws_neutral_hint` |
| G5 | PDF 页边距：PDF 夹取 5–60 mm，配置/设置允许 0–50 mm，0–5 mm 被静默提升 | 统一为 **5–50 mm**：`EXPORT_MARGIN_MM_MIN/MAX` 单一常量，配置解析夹取、设置字段范围与 apply 夹取、`PageGeom::a4` 全部共用；设置弹窗标签同步为 5–50 | `crates/veil-ui/src/theme.rs`、`view/settings.rs`、`crates/veil/src/platform/pdf_layout.rs::PageGeom::a4` | `theme.rs::export_margin_clamps_to_shared_pdf_interval`、`view/settings.rs::export_margin_field_range_matches_pdf_interval`、`pdf_layout.rs::page_margin_clamps_to_shared_interval` |

G5 选型理由：保留 PDF 的 5 mm 下限（防止内容贴边/落入打印机不可打印区），把配置与设置 UI 的下限从 0 提升到 5；若允许 0 mm，则多数打印机实际无法输出，等于放开无效版式。

## 三、延后缺口（未在本轮修复）

| 编号 | 缺口 | 位置 | 延后理由 |
|---|---|---|---|
| G4 | 大纲/快速打开/历史面板无“加载”态，超大目录/快照列表会阻塞 UI | 见矩阵“加载”行 | 需要异步加载态（后台任务 + 进度/占位态），属功能增量而非视觉层修补；本轮只关闭成本可控的提示类缺口 |

除以上延后项外，矩阵中的“现有实现”与本规格一致。

## 四、PDF 视觉对齐核验结论

| 项 | 规格 | 现状（已核验） | 结论 |
|---|---|---|---|
| 字号 | 标题与正文从 `veil_ui::type_scale` 派生 | 标题 `heading_size` → `type_scale::paper_heading_size`；正文由本地 `11.0` 改为 `type_scale::PAPER_BODY_PX`；代码/表格由 `type_scale::paper_code_size` / `paper_table_size` 派生（9.625 / 8.8，同源 `type_scale`） | 正文/标题/代码/表格全部同源 |
| 页边距 | 来自配置 `export_margin_mm` | 打印与 PDF 均 `PageGeom::a4(session.export_margin_mm())`，`session.export_margin_mm()` 读 `config.export_margin_mm`；配置解析、设置 UI 与 `PageGeom` 共用 `EXPORT_MARGIN_MM_MIN/MAX = 5–50` | 一致（原 G5 已关闭） |
| 页码 | 受配置开关控制 | `LayoutOpts.page_numbers = session.export_options().page_numbers`（读 `config.export_page_numbers`），`append_footers(opts.page_numbers)` 决定是否写 `n / total` | 一致（默认关闭） |
| 线条 | 取 hairline / rail 语义 | 表格网格与分隔线由自由值 0.6/0.8 pt 改为 `HAIRLINE_PT = type_scale::hairline × 72/96 = 0.75 pt`；引用/Callout 竖条取 `type_scale::rail`（3.0，数值不变） | 已对齐语义；单位按 96 dpi→pt 换算，未改版式节奏 |

对应测试：`crates/veil/src/platform/pdf_layout.rs::paper_fonts_and_strokes_come_from_type_scale`。

## 五、空态去红验收点

| 位置 | 改动 | 测试 |
|---|---|---|
| `crates/veil-ui/src/tokens.rs::state::empty_fg` | `theme.danger` → `theme.muted` | `tokens::tests::state_matrix_has_single_source_and_distinct_states` 断言 `empty_fg == muted` 且 `!= danger`（浅/深/高对比度） |
| 查找无命中计数 | 随 `empty_fg` 改为中性色；注释同步 | `crates/veil-ui/src/app_tests/view.rs::search_bar_no_match_uses_neutral_and_disabled_colors` |
| 非法正则查询串 | 保持 `theme.danger`（真错误） | `view/search_view.rs::paint_search` |
| 版本历史错误、错误模态、危险按钮 | 保持 `theme.danger`（真错误/危险操作） | 既有用例 `version_history_delete_and_clear_go_through_danger_modal` 等 |
