# Veil 完美表面事实底座（perfect-surface audit）

> 日期：2026-11-27｜性质：只读审计｜基线：HEAD `1e6ae40`（`fix(print): keep short code blocks on one page`），版本 `3.0.2`（`Cargo.toml:12`），审计开始时工作区 clean（审计期间出现非本审计的并行修改，见 §0.2）。
> 本文件是后续修复任务的唯一权威清单；未修改任何源码、README、`.trae` 文件，也未提交。唯一新增文件为本文件。
> 本文件不宣称"绝对完美"：只把每条已知不完美项落到**可追溯证据 + 唯一归因 + 可勾选关闭判据**。

## 0. 审计口径

### 0.1 事实来源

- `.trae/specs/converge-perfect-writing-surface/{spec,tasks,checklist}.md`（本轮任务边界与 Task 2–7 归属）
- `docs/review/jobs-lens-report-v2.md`（十维 41/50 与扣分理由）
- `docs/review/evidence/layout-consistency-2026-11-26.md`（屏幕/PDF 断行与分页接受项）
- `docs/qa/report-2026-11-27.md`、`docs/qa/release-facts-2026-11-27.md`、`docs/qa/w11-load-state-measurement.md`、`docs/qa/w10-output-pipeline-evidence.md`
- `README.zh-CN.md:139-157`、`README.md:139-156`（已知限制）
- `docs/adr/ADR-011-wgpu-deferral.md:34-42`（GPU 四个触发条件）
- `.trae/specs/**/{tasks,checklist}.md`（历史规格未勾选项统计）
- 源码：`crates/veil/src/platform/pdf_layout.rs`、`crates/veil-ui/src/print.rs`、`crates/veil-ui/src/view/table_strategy.rs`、`crates/veil-mermaid-offline/src/lib.rs`

### 0.2 只读与证据等级

- 所有"事实"均能追到源码行、文档行或命令输出；无法证实的判断在 §5 标为"待验证"，不写成本轮结论。
- 审计期间工作区出现 `crates/veil/src/platform/pdf_layout.rs` 的并行修改（`git diff --numstat` = +281/−106，非本审计产生）；本文件引用该文件的行号对应审计读取时的 HEAD `1e6ae40` 版本，若并行修改落地，需按函数名复核行号。
- 历史文档中的数字（如 296.55 ms）保留原文；后续口径更正（281.10 ms、改称合成参考指标）单独注明出处，不覆盖历史记录。
- `docs/qa/report-2026-11-27.md` 由 `cargo test -p veil --test syntax_matrix` 重新生成，禁止手工改写；本文件只引用、不修改。

### 0.3 历史规格未勾选项统计方法

统计对象为 `.trae/specs` 下每个目录的 `tasks.md` 与 `checklist.md`，计数规则为**所有 `[ ]` 出现次数（含缩进嵌套项）**：

```powershell
$root='d:\VSCode\Veil\.trae\specs'
Get-ChildItem $root -Directory | ForEach-Object {
  $d=$_.Name; $n=0
  foreach($f in @("$($_.FullName)\tasks.md","$($_.FullName)\checklist.md")){
    if(Test-Path $f){ $n += (Select-String -Path $f -Pattern '\[ \]' -AllMatches).Count }
  }
  if($n -gt 0){ "$d`t$n" }
}
```

实测结果（2026-11-27）：

- `.trae/specs` 共 **101** 个目录。
- 含未勾选项的目录 **28** 个，其中当前规格 `converge-perfect-writing-surface` **73** 条。
- **历史目录 27 个，共 662 条未勾选**；逐目录裁决见 §2。
- 与 Task 1.2 自述的"41 个"不一致，差异登记为 F-024；不修改 `.trae`，以本文件实测数为准。

### 0.4 台账总览

- F 条目总数：**34**（F-001 … F-034）。
- 严重度：**P0 = 0、P1 = 15、P2 = 14、P3 = 5**。
- 归因：**已修复 7、本轮修复 13、接受并说明 9、延后并说明 3、测量闭合 2**（合计 34；F-025/F-029 经复核由「延后」改判为「已修复：文档更正/非目标」；F-015/F-016 于 2026-11-28 由 Task 5 测量闭合）。
- P0 为 0 的含义：本轮未发现"已证实的数据丢失、崩溃或安全问题"；已知缺陷集中在分页一致性、矩阵覆盖与证据闭合，不把未取证项写成 P0。

---

## 1. 缺陷与不完美项台账

### 1.1 语法矩阵覆盖边界

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-001 | 块级相邻只覆盖空行边界 | ADJB 恒以 `A + "\n\n" + B` 构造，未枚举 `A + "\n" + B` 的单换行边界 | `docs/qa/report-2026-11-27.md:111`；`.trae/specs/cover-adjacent-pairs-and-deferred-items/spec.md:11` | P1 | 本轮修复 | Task 2.1 |
| F-002 | 行内相邻只覆盖空格/无空格 | ADJI 为 12×12×2，边界只有 `A + " " + B` 与 `A + B`，未覆盖换行边界与 `。`、`，`、`）`、`.`、`,` 紧邻 | `docs/qa/report-2026-11-27.md:117`；`cover-adjacent-pairs-and-deferred-items/spec.md:11` | P1 | 本轮修复 | Task 2.2 |
| F-003 | 容器内只测一个内层块 | ADJC 为 5 类容器 × 18 个块单元，每个用例只放一个内层块，未测两个内层块相邻 | `docs/qa/report-2026-11-27.md:106`；`cover-adjacent-pairs-and-deferred-items/spec.md:12` | P1 | 本轮修复 | Task 2.3 |
| F-004 | 未枚举嵌套容器 | 现有组只有 ADJB/ADJI/ADJC，报告与规格均未登记"容器套容器"（ListItem/BlockQuote/Details 3×3）组合。**已闭合（2026-11-28 复核）**：ADJX 9 条容器套容器用例全绿；其中嵌套 `<details>` 已由 Task 2 识别为 Details 块，不再锁定降级形态 | `docs/qa/report-2026-11-27.md:106-108`；`cover-adjacent-pairs-and-deferred-items/spec.md:11-13` | P1 | 本轮修复（已闭合） | Task 2.4 / 2.1 |

### 1.2 分页与输出一致性

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-005 | 原生 PDF 表格续页不重复表头 | 表格行序列只把 header 放一次；续页只重画边框，表头背景仅在 `ri == 0 && first_chunk` 绘制，续页无表头文字 | `crates/veil/src/platform/pdf_layout.rs:1449-1453`、`1499`、`1517-1525`；`docs/review/evidence/layout-consistency-2026-11-26.md:84` | P1 | 本轮修复 | Task 3.1 |
| F-006 | 超高表格行仍从行中间拆开 | 整行高度 ≤ 一页时优先整体移页；行高超过一页可用高时，`fit` 按行数切片跨页，行内文字被拆到两页 | `crates/veil/src/platform/pdf_layout.rs:1491-1514` | P2 | 本轮修复 | Task 3.1/3.2 |
| F-007 | 打印路径把网格表格当原子块，长表硬切 | `compute_page_breaks` 对渲染态表格只保留块顶候选；无内部断点时执行 `breaks.push(limit)`，在页底硬切 | `crates/veil-ui/src/print.rs:322-324`、`336-339`、`371-378` | P1 | 本轮修复 | Task 3.2 |
| F-008 | 打印与原生 PDF 表格分页规则不一致 | PDF 按表格行/行内切片分页并逐页重画边框；打印把表格视作原子块，只能整块移页或硬切 | `crates/veil/src/platform/pdf_layout.rs:1499-1587`；`crates/veil-ui/src/print.rs:336-339`、`371-378` | P1 | 本轮修复 | Task 3.2 |
| F-009 | 超高原子块两条路径策略不一致 | 历史：PDF 对图片/公式/图表限高并等比缩放（图片上限 `content_h * 0.8`，Mermaid 上限 `content_h - 12pt`），对表格按行分页；打印对表格/公式图只保留块顶候选，超页硬切。**已修复（Task 3）**：显示公式在两条路径都等比缩放到一页可用高内（打印 80% 页帧高、PDF `content_h - 12pt`），块高/绘制/命中同口径；打印 `print_scales_tall_display_math_to_one_page`、PDF `tall_display_math_scales_to_single_page_content_height` 锁定（2026-11-28 复核：两单测随 `cargo test --workspace` 通过） | `crates/veil/src/platform/pdf_layout.rs:1130-1163`；`crates/veil-ui/src/interaction.rs`（`math_display_size`/`math_print_scale`）、`crates/veil-ui/src/paint.rs:1345-1360`；`crates/veil-ui/src/print.rs`（R4） | P1 | 本轮修复 | Task 3.3 |

### 1.3 WYSIWYG 复杂内容

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-010 | 宽表触底后向右溢出并被裁剪，无横向滚动 | 策略层注释明确"触底仍超宽时表格向右溢出，由 frame 裁剪（v1 不做横向滚动）"；列宽下限 `MIN_COL_WIDTH = 40.0` | `crates/veil-ui/src/view/table_strategy.rs:8-9`、`33` | P1 | 本轮修复 | Task 4.1 |
| F-011 | Mermaid 深色只自动覆盖三类，其余 16 图型未验证 | 暗色 `themeCSS` 只覆盖 `flowchart-v2`、`mindmap`、`gantt`；自动对比度测试覆盖上述三类 + `c4context` stereotype；其余 16 种仅校验几何/可见文字/出图成功，逐图人工目检为 NOT RUN | `crates/veil-mermaid-offline/src/lib.rs:179-196`、`385`、`570`、`589`、`676`；`README.zh-CN.md:144`；`docs/review/jobs-lens-cross-surface.md:61-67` | P2 | 本轮修复 | Task 4.2 |

### 1.4 证据 NOT RUN 与测量缺口

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-012 | 物理打印机 NOT RUN | 打印只通过 Microsoft Print to PDF 与共用 PDF 分页路径验证，未在实体打印机出纸；矩阵报告明确物理打印不在断言口径内 | `README.zh-CN.md:154`；`docs/qa/report-2026-11-27.md:101`；`docs/qa/w10-output-pipeline-evidence.md:21` | P1 | 延后并说明 | Task 6.2 |
| F-013 | 系统 DPI 100/125/200 NOT RUN | 仅 150% 有自动首帧检查；100/125/200 未实测，`README` 与十维报告均记为 NOT RUN | `README.zh-CN.md:154`；`docs/review/jobs-lens-report-v2.md:67`、`270` | P1 | 延后并说明 | Task 6.1 |
| F-014 | 真机中文 IME 组合与候选窗 NOT RUN | 代码已修 IME 锚点与 preedit 绘制（端口单测通过），但微软拼音/五笔真机六步走查未执行，不能宣称 PASS | `docs/review/jobs-lens-report-v2.md:337-341`、`357-358`；`README.zh-CN.md:164` | P1 | 延后并说明 | Task 6.3 |
| F-015 | 版本历史面板未单独实测 | w11 只实测快速打开（5000 文件约 13 ms）与大纲（0.34 ms）；历史面板"未单独实测"，沿用"可能短暂阻塞"的表述 | `docs/qa/w11-load-state-measurement.md:62-66`、`74-75` | P2 | 延后并说明 | 不修 |
| F-016 | 5000 标题解析预算只有单次 release 样本，无 p95 | `Session::open(5000 headings)` release 只有 148.385 ms 单次样本；目录枚举/候选构建有 n=20 与 p95，但该路径没有；预算登记为 ≤200 ms | `docs/qa/w11-load-state-measurement.md:40-47`、`68-70` | P2 | 延后并说明 | 不修 |

### 1.5 产品状态、登记与发布事实

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-017 | 两份 PDF 历史件并存 | `veil-export.pdf`（修复前 10,262,757 B）与 `veil-export-compressed.pdf`（341,076 B）同目录保留，用于对照，不覆盖历史件 | `docs/qa/w10-output-pipeline-evidence.md:66-73`、`89`；`docs/qa/evidence/w10-2026-11-27/` 实存两份 | P3 | 接受并说明 | 不修 |
| F-018 | 历史 spec 未勾选项此前未审计 | 101 个 spec 目录中 28 个含未勾选（含当前规格 73 条），历史 27 个目录共 662 条；本文件 §2 已逐目录登记裁决，只登记不删除 | 本文件 §0.3、§2 | P2 | 已修复（本次审计） | Task 1.2 |
| F-019 | 未推送 GitHub | release-facts 明示未推送、未打 tag；工作区 clean | `docs/qa/release-facts-2026-11-27.md:7`；`git status --porcelain` 无输出 | P3 | 接受并说明 | 不修 |
| F-020 | 版本号未改 | 根 `Cargo.toml` 仍为 `3.0.2`；release-facts 明示未修改版本号 | `Cargo.toml:12`；`docs/qa/release-facts-2026-11-27.md:7` | P3 | 接受并说明 | 不修 |
| F-021 | `.trae` 不进 git | `.gitignore` 明确忽略整个 `.trae/` | `.gitignore:28` | P3 | 接受并说明 | 不修 |

### 1.6 本次审计新发现

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-022 | 英文 README 的 Mermaid 对比度条目混用深/浅色覆盖范围 | `README.md:144` 在 "Dark Mermaid contrast" 条目下称 WCAG 断言覆盖 mindmap、kanban、journey、gantt、c4context；代码中 kanban/journey 的对比度测试是**浅色**主题（`kanban_light_...`、`journey_light_...`），深色 `themeCSS` 不含这两类；中文 README 明确写了"浅色 override、无对应深色 override" | `README.md:144`；`README.zh-CN.md:144`；`crates/veil-mermaid-offline/src/lib.rs:197-203`、`2024`、`2050` | P2 | 本轮修复（文档收口） | Task 7.1 |
| F-023 | README「高对比度」仍写 NOT RUN，与当前证据冲突 | 中英文 README 的未验证项仍称"真实高对比度主题下是否触发、观感"均未验证；但十维报告记录真实系统切换 PASS（228 ms），release-facts 记录强制高对比度复验截图 | `README.zh-CN.md:154`；`README.md:153`；`docs/review/jobs-lens-report-v2.md:145`；`docs/qa/release-facts-2026-11-27.md:64` | P2 | 本轮修复（文档收口） | Task 7.1 |
| F-024 | Task 1.2 自述"41 个历史目录"与实测不符 | `tasks.md` 写"41 个含未勾选项的历史目录"；实测 101 个 spec 目录、28 个含未勾选、其中历史 27 个（当前规格另计）。不修改 `.trae`，以本文件 §0.3 实测为准 | `.trae/specs/converge-perfect-writing-surface/tasks.md:7`；本文件 §0.3 | P3 | 接受并说明 | 不修 |
| F-025 | README 仍宣称已移除的 AccessKit/UIA 能力 | AccessKit/UIA 层已于 `3a6d72d` 永久移除（删 `crates/veil-ui/src/accessibility.rs` 666 行，`Cargo.lock` 无 accesskit），但中英 README 的「无障碍」章节仍写 `accessibility` feature、UIA 能力清单与关闭命令，属文档失实 | `git show --stat 3a6d72d`；`rg -n "accesskit" Cargo.lock crates`（源码无命中）；修正前 `README.zh-CN.md:159-167`、`README.md:158-166` | P1 | 已修复（本轮 README 改为「不提供屏幕阅读器支持」，删除失效构建命令） | Task 5.4 / 7.1 |

### 1.7 已接受或延后的已知边界

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-026 | PDF/打印中的远程图片仍降级为 alt 或 `[图片]` | HTML 导出改为可见警告，但 PDF/打印不下载远程资源，仍 alt 降级；属离线契约下的已知降级 | `README.zh-CN.md:147`；`docs/review/jobs-lens-cross-surface.md:43`；`docs/review/jobs-lens-report-v2.md:271` | P2 | 接受并说明 | 不修 |
| F-027 | 屏幕与 PDF 断行存在 11 行已归因差异 | 产品宽度下 4 组语料共 11 行内容不同，原因固定为容量、半角禁则表差、空格偏好、字宽；无丢字、越界、行首禁则违规 | `docs/review/evidence/layout-consistency-2026-11-26.md:13`、`66-75`、`89-98` | P2 | 接受并说明 | 不修 |
| F-028 | `Ctrl+Shift+P` 在 AltGr 布局下可能与字符输入冲突 | 已知键盘布局取舍，已注明 | `docs/review/jobs-lens-report-v2.md:275`（引用 `crates/veil-ui/src/input.rs:125`） | P2 | 接受并说明 | 不修 |
| F-029 | 无障碍正文 8000 字节快照的边界描述已失效 | 该边界属于已移除的 AccessKit 层；现产品不提供任何 UIA 语义树，原「快照范围」讨论不再适用，改为「无障碍为非目标」登记 | `git show --stat 3a6d72d`；`docs/review/deferred-and-manual-register-2026-11-27.md` §1.4 | P2 | 已修复（改判为非目标） | 不修 |
| F-030 | 5005 块全量排版 296.55/281.10 ms 未达原 <100 ms 门禁 | 分段实测冷启动 281.10 ms，逐块 shaping 238.46 ms（占 84.8%）；已改称"合成参考指标"，不再作为发布门禁；真实交互脏块重排 271 µs | `docs/review/evidence/perf/current-head-2026-11-26.md:23`、`58-81`；`docs/review/jobs-lens-report-v2.md:344-346` | P2 | 接受并说明 | 不修 |

### 1.8 已修复项（保留在台账，防止回退）

| ID | 标题 | 当前事实 | 证据位置 | 严重度 | 归因 | 归属任务 |
|---|---|---|---|---|---|---|
| F-031 | HTML 导出的 `defs` 前缀把缩进代码降级为段落 | 已在 `defs` 与正文之间补空行，覆盖普通块/列表项/Callout/标题四处拼接点；结构断言全量恢复 | `docs/qa/report-2026-11-27.md:62-66`、`89` | P1 | 已修复 | 不修 |
| F-032 | 相邻高亮的 4 连等号边界被整段降级 | `try_highlight` 增加"4 连等号边界"规则，现解析出两个 Highlight 与两个 `<mark>` | `docs/qa/report-2026-11-27.md:66`、`90` | P2 | 已修复 | 不修 |
| F-033 | 原生 PDF 未压缩位图导致整档 10.26 MB | 改为 Flate + PNG predictor 后整档 341,076 B（约缩小 30.1 倍） | `docs/qa/w10-output-pipeline-evidence.md:69-73`、`78-89`；`docs/qa/release-facts-2026-11-27.md:39` | P2 | 已修复 | 不修 |
| F-034 | 短代码块被页底拆开 | 改为整块 ≤ 一页时整体移页；打印件 6 行代码全部落在第 3 页 | `docs/qa/w10-output-pipeline-evidence.md:30-33`；`docs/qa/release-facts-2026-11-27.md:38`；`git log -1` = `1e6ae40` | P1 | 已修复 | 不修 |

---

## 2. 历史规格目录裁决

> 口径：只登记，不删除、不修改 `.trae` 任何文件。裁决依据仅使用可观察证据：目录内 spec 日期/显式状态、后续 change-id 的重叠、对应代码或文档是否已存在。"证据不足"统一写"仍有效（待裁决）"。

| 目录 | 未勾选 | 裁决 | 依据 |
|---|---:|---|---|
| `audit-render-interaction-defects` | 1 | 仍有效（待裁决） | 2026-11-14 v1.5.1 真机缺陷批次；剩余 1 条为设置「上次窗口几何」行，未找到对应的完成记录 |
| `audit-veil-exhaustive-matrix` | 128 | 应归档 | spec 自述"状态：只读归档……不得作为待办来源"（`spec.md` 首段） |
| `close-ime-and-feedback-gaps` | 10 | 仍有效 | 2026-11-26 基线 `e9aeb20`；IME/反馈缺口与十维报告 §10.1 的"真机仍 NOT RUN"直接对应 |
| `close-pdf-and-plan-wave5` | 1 | 仍有效（待用户复评） | 唯一未勾选项是 AC-9「观感评分 ≥4（用户复评）」，等待用户回填；PDF 工作已由 wave4/wave5 与 `close-render-output-gaps` 推进，但该验收签字未闭合 |
| `converge-dual-binary-excellence` | 9 | 已被后续规格覆盖 | 后续 `converge-single-veil-surface`（2026-11-20）与 README/release-facts 的单 `veil.exe` 产品故事取代双版本方向 |
| `converge-single-veil-surface` | 11 | 仍有效 | 未勾选项集中在真实 GitHub Actions、Linux runner、100% DPI、实体打印等 needs-user 项；当前仍是 NOT RUN |
| `cut-first-frame-to-100ms` | 23 | 已被后续规格覆盖 | 十维报告 §4.2 与 `current-head-2026-11-26.md:28` 记录端到端首帧 84.8 ms PASS；Phase 2 已重测 |
| `decide-v1-5-wave` | 40 | 应归档 | 2026-11-13 的 v1.5 决策输入；当前版本 3.0.2，后续规格已覆盖该决策周期 |
| `drive-v1-excellence` | 57 | 应归档 | spec 自述"状态：提案，未执行……等待批准后进入 apply" |
| `expand-editor-scope` | 61 | 应归档 | 2026-11-09 的 v1.2/v1.3 三地平线规划，已被后续 v1.5–v3.0 规格与实际交付取代 |
| `extend-n10-top-tier-html` | 3 | 仍有效 | 未勾选项是行内 `<details>` 折叠与行内大图尺寸策略，明确写"待立项/已知限制"，尚未被后续规格覆盖 |
| `file-icons-and-association` | 1 | 仍有效（待人工） | 未勾选项需用户在系统设置把 `.md` 设为 Veil 后截图，属人工证据缺口 |
| `find-and-highlight` | 39 | 已被后续规格覆盖 | 查找替换能力已交付，README 记录区分大小写、正则、捕获组替换等（`README.zh-CN.md:108`）；未勾选为台账滞后 |
| `own-line-layout-policy` | 54 | 已被后续规格覆盖 | `source_plain_lines` 纯文本行视图已实现并有测试（`crates/veil-ui/src/app_tests/app.rs:2754-2781`），README 记录两种源码排版（`README.zh-CN.md:157`） |
| `plan-remaining-backlog` | 1 | 仍有效（待裁决） | 唯一未勾选项是"推翻 D10⑤ HTML 白名单不做裁决"，未找到后续明确裁决记录 |
| `polish-dual-binary-editor-quality` | 1 | 已被后续规格覆盖 | 双版本集成验证随 `converge-single-veil-surface` 的单产物方向失效 |
| `polish-floating-bar-s3-s4-f2` | 1 | 仍有效（待取证） | 唯一未勾选项是"深色主题截图未取证"；代码走同一 Theme/state 路径，但视觉证据仍缺 |
| `polish-perceived-quality` | 4 | 仍有效（待用户确认） | 未勾选项为用户确认类（对比证据、≥2 评分、定位句复述），需用户在场 |
| `polish-veil-ux-craft` | 8 | 仍有效 | 未勾选项为 100% DPI、125%/200% DPI、真实中文 IME、实体打印等 needs-user 项，与十维报告一致 |
| `remediate-product-trust-and-accessibility` | 92 | 仍有效 | spec 自述"P1（Task 4-8）和 P2（Task 9-10）待执行"；仅 P0 已完成 |
| `repair-launch-first-frame-crash` | 1 | 仍有效（待真机视觉复验） | 关闭与无 panic 已验证；键入与首帧黑块视觉仍 NOT RUN（checklist 原文） |
| `replace-content-components` | 5 | 已被后续规格覆盖 | 代码高亮已切到 `syntect`（`crates/veil-ui/Cargo.toml:15`），组件替换已落地 |
| `ship-veil-clean-release` | 2 | 仍有效 | 未勾选项是 `.trae` 非 spec 大文件清理；release-facts 记载仍未 push，仓库清理决策未闭合 |
| `unify-editor-backlog` | 52 | 应归档 | 2026-11-10 的统一台账，已被 `veil-top-tier-roadmap` 与后续执行规格取代 |
| `ux-polish-consistency` | 1 | 仍有效（待人工） | 唯一未勾选项是"纯键盘触发 + 截图的对话框人工复测（不阻塞本轮交付）" |
| `veil-top-tier-roadmap` | 16 | 应归档 | 2026-11-10 的路线图，后续由 `plan-remaining-backlog` 与 v3.0 系列规格接管 |
| `walkthrough-and-landing` | 40 | 应归档 | 2026-11-06 的真机走查批次，已被后续 converge/release 走查规格与 `docs/qa/` 证据取代 |

**分布**：仍有效 **14**（其中 6 条标注"待裁决/待取证/待用户/待人工"）、已被后续规格覆盖 **6**、应归档 **7**，合计 **27**。

---

## 3. 四类归因汇总

> 约定：**本轮修复** = 由 `.trae/specs/converge-perfect-writing-surface/tasks.md` 的 Task 2/3/4/5 覆盖。F-022、F-023 属 Task 7.1 文档收口，单列注明，不计入 Task 2–5 代码修复。

### 3.1 已修复（5）

| ID | 关闭事实 |
|---|---|
| F-018 | 本次审计完成历史规格逐目录登记（§2），只登记不删除 |
| F-031 | ADJB-086/239 的 HTML `defs` 前缀缺陷已修，结构断言恢复 |
| F-032 | ADJI-184 的 4 连等号高亮缺陷已修 |
| F-033 | PDF 未压缩位图已修，整档 341,076 B |
| F-034 | 短代码块整块移页已修，打印件验证通过 |

### 3.2 本轮修复（13）

| ID | 归属 | 说明 |
|---|---|---|
| F-001 | Task 2.1 | 块级单换行边界 18×18 |
| F-002 | Task 2.2 | 行内换行/标点紧邻边界 |
| F-003 | Task 2.3 | 容器内两个内层块相邻 |
| F-004 | Task 2.4 | 嵌套容器 3×3 |
| F-005 | Task 3.1 | PDF 表格续页重复表头 |
| F-006 | Task 3.1/3.2 | 表格行边界断点，禁止行中间硬切 |
| F-007 | Task 3.2 | 打印长表按行断点 |
| F-008 | Task 3.2 | 打印与原生 PDF 表格分页同规则 |
| F-009 | Task 3.3 | 超高原子块单一策略 |
| F-010 | Task 4.1 | 宽表横向滚动或等价方案 |
| F-011 | Task 4.2 | Mermaid 16 图型深色补证 |
| F-022 | Task 7.1（文档收口） | 英文 README 深/浅色覆盖范围澄清 |
| F-023 | Task 7.1（文档收口） | README 高对比度状态与证据同步 |

### 3.3 接受并说明（9）

| ID | 接受理由 | 书面位置 |
|---|---|---|
| F-017 | 历史 PDF 保留用于对照，不覆盖历史件 | `docs/qa/w10-output-pipeline-evidence.md:89`；本文件 F-017 |
| F-019 | 本轮不推送 GitHub、不打 tag | `docs/qa/release-facts-2026-11-27.md:7`；本文件 F-019 |
| F-020 | 本轮不改版本号，仍为 3.0.2 | `Cargo.toml:12`；`docs/qa/release-facts-2026-11-27.md:7` |
| F-021 | `.trae` 是本地 agent 工作区，按设计不入库 | `.gitignore:28` |
| F-024 | 不修改 `.trae`；以本文件实测数（27 个历史目录 / 662 条）为准 | 本文件 §0.3、F-024 |
| F-026 | 导出/PDF 永不下载远程资源是离线契约；降级已披露 | `README.zh-CN.md:147`；`docs/review/jobs-lens-cross-surface.md:43` |
| F-027 | 11 行差异有固定原因，无丢字/越界/禁则违规；媒介差异接受 | `docs/review/evidence/layout-consistency-2026-11-26.md:89-98` |
| F-028 | AltGr 布局冲突是已记录的键盘取舍 | `docs/review/jobs-lens-report-v2.md:275` |
| F-030 | 全量排版改称合成参考指标，不作发布门禁；真实交互脏块重排 271 µs | `docs/review/evidence/perf/current-head-2026-11-26.md:74-81` |

### 3.4 延后与已闭合项（F-015 / F-016 已于 2026-11-28 闭合）

| ID | 延后理由 | 重开判据 |
|---|---|---|
| F-012 | 物理打印机需要硬件与人工出纸 | Task 6.2 步骤执行并有样张 |
| F-013 | DPI 100/125/200 需真实显示环境切换与截图 | Task 6.1 给出截图、首帧、命中与版心一致性 |
| F-014 | 真机 IME 需人在场走微软拼音/五笔六步 | Task 6.3 六步走查通过并留证 |
| F-015 | **已闭合（2026-11-28，Task 5）**：版本历史面板 5000 快照已同口径测量，打开 p95 10.738 ms、滚动 p95 3.642 ms、关闭 p95 0.423 ms | `docs/qa/perf-distribution-2026-11-27.md` §1；`probe-gate-stability-{1,2,3}.md` |
| F-016 | **已闭合（2026-11-28，Task 5）**：5000 标题 `open_ms` 为 n=9 分布，p95 96.463 ms（本轮 probe-gate 复跑 100.234 ms），登记预算 ≤200 ms | `docs/qa/perf-distribution-2026-11-27.md` §1；`docs/qa/probe-gate-2026-11-28.md` |
| F-025 | README 与源码不一致（AccessKit 已移除仍在宣传） | 中英 README 更正为「不提供屏幕阅读器支持」并删除失效构建命令（本轮已执行） |
| F-029 | 该边界随 AccessKit 移除而失效 | 登记为「无障碍是非目标」，不再保留 8000 字节快照的待办表述 |

---

## 4. 收口清单

> 每条对应一个 F 编号的最终关闭判据。要求：不存在"未处理且未解释"的条目；"修好"必须有测试名或证据文件，"接受/延后"必须指向 §3.3/§3.4 的理由。

### 4.1 本轮修复关闭判据

- [x] **F-001**：`ADJN` 306 执行 / 18 N/A，逐对块序列见 `report-2026-11-27.md`；NOT RUN = 0。
- [x] **F-002**：`ADJI2` 864 条（换行 + `。`、`，`、`）`、`.`、`,`），七表面与摘要均接入。
- [x] **F-003**：`ADJC2` 110 条（5×18 自相邻 + 5×4 跨类型），断言互不吞并与容器归属。
- [x] **F-004**：`ADJX` 9 条（3×3），层级/闭合/归属断言通过；嵌套 `<details>` 已由 Task 2 修复为 Details 块，原「2 条降级形态」登记作废（2026-11-28 复核，`cargo test --workspace` 通过）。
- [x] **F-005**：PDF `repeat_table_header` + `table_paginates_on_row_boundaries_and_repeats_header` 断言续页表头文字。
- [x] **F-006**：两条路径都只在整行边界断页；单行自身高于一页时才允许行内硬切（规则与测试见 Task 3 单测）。
- [x] **F-007**：打印 `compute_page_breaks` 加入表格行候选，`print_table_page_breaks_land_on_row_tops_and_reserve_header` 锁定。
- [x] **F-008**：两路径同规则（整行断页 + 续页表头）；差异仅剩显式尺寸超页原子块的硬切，已在 `print.rs` R4 注释与 §3.3 书面登记。
- [x] **F-009**：表格整行断页；独立图片/Mermaid 两路径都缩放到一页内（PDF 显式缩放，打印受 `image_max_height` 80% 页高上限）。**已闭合**：高于一页的显示公式改为两路径同规则等比缩放到一页可用高内（打印 80% 页帧高、PDF `content_h - 12pt`），不丢内容、不跨页硬切；单测 `print_scales_tall_display_math_to_one_page` 与 `tall_display_math_scales_to_single_page_content_height`。历史差异与重开判据保留在 register §2。**2026-11-28 Task 6 复核通过**：两单测随 `cargo test --workspace` 执行，无失败。
- [x] **F-010**：`Shift+滚轮` 可读宽表平移，`narrow_window_wide_table_readable_via_shift_wheel_pan` 锁定绘制/命中/光标一致性。
- [x] **F-011**：22 类深色逐图目检 + 证据目录；`dark_corpus_renders_differs_and_rasterizes_opaque`；c4 `Rel` 已按 aria 限定覆盖；gitGraph 3.35:1 记为轻微限制。
- [x] **F-022**：中英 README 已按「22 类目检 + 自动断言 + 两处轻微限制」重写，与 `lib.rs` 一致。
- [x] **F-023**：中英 README 高对比度条目已改为「真实系统切换 PASS（228 ms）+ 强制高对比度截图，极端主题待人确认」。

### 4.2 已修复回归看护

- [x] **F-018**：§2 的 27 个历史目录裁决保持只登记不删除；本轮未改动 `.trae`。
- [x] **F-031**：`CASE-ADJB-086`、`CASE-ADJB-239` 持续全绿（`cargo test -p veil --test syntax_matrix`，2148 用例 0 FAIL）。
- [x] **F-032**：`CASE-ADJI-184` 的 `<mark>` 与 `inline_kinds` 断言持续全绿。
- [x] **F-033**：`cargo test -p veil --lib platform::pdf::tests` 通过，整档低于 1 MiB 预算。
- [x] **F-034**：本轮未改打印几何的代码块路径；`verify_print_codeblock.py` 未重跑，其判定对象（6 行代码同页）由 `print_keeps_short_code_block_on_one_page` 单测持续看护。

### 4.3 接受项关闭判据

- [x] **F-017**：两份 PDF 并存，本轮未删除历史件；用途见 §3.3。
- [x] **F-019**：release-facts 记录「未推送、未打 tag」；本轮仍不推送。
- [x] **F-020**：版本号保持 3.0.2，release-facts 已更新。
- [x] **F-021**：`.gitignore` 继续忽略 `.trae/`，本轮提交不含 `.trae`。
- [x] **F-024**：本文件 §0.3 实测数（27 个历史目录 / 662 条）作为后续口径。
- [x] **F-026**：README 保留「PDF/打印永不下载远程资源」的离线契约说明，本轮未改。
- [x] **F-027**：`cargo test -p veil pdf_layout` 通过，11 行差异归因测试保持。
- [x] **F-028**：`Ctrl+Shift+P` 的 AltGr 取舍文档保留，本轮未改键位。
- [x] **F-030**：全量排版仍只作参考指标；本轮未重开性能门禁。

### 4.4 延后项关闭判据

- [x] **F-012**：已登记为人工项，步骤与判定见 `docs/review/deferred-and-manual-register-2026-11-27.md` §1.2；保持 NOT RUN。
- [x] **F-013**：已登记为人工项，步骤与判定见 register §1.1；保持 NOT RUN。
- [x] **F-014**：已登记为人工项，六步与判定见 register §1.3；保持 NOT RUN。
- [x] **F-015**：已闭合（2026-11-28）：版本历史面板 5000 快照同口径实测，打开 p95 10.738 ms、滚动 p95 3.642 ms、关闭 p95 0.423 ms，见 `docs/qa/perf-distribution-2026-11-27.md` §1。
- [x] **F-016**：已闭合（2026-11-28）：5000 标题 `open_ms` n=9 分布 p95 96.463 ms（probe-gate 复跑 96.199 ms），低于 ≤200 ms 预算，见同上 §1。
- [x] **F-025**：README 中英「无障碍」章节已改为「不提供屏幕阅读器 / UIA 支持（AccessKit 于 `3a6d72d` 移除）」，失效构建命令已删除。
- [x] **F-029**：已改判为「无障碍是非目标」，不再作为证据缺口或待办。

---

## 5. 待验证与证据边界

以下项目在 2026-11-27 审计时**没有**取得足以写成结论的新证据，保持"待验证/NOT RUN"，不得在后续文档中改写成 PASS（2026-11-28 新增的测量与闭合已在对应条目注明）：

1. ~~16 种 Mermaid 图型的深色观感与 c4 `Rel` 连线标签对比度~~：已由 Task 4.2 关闭（22 类逐图目检 + 证据目录 + 自动断言；c4 已按 aria 限定覆盖）。**gitGraph 分支标签已于 2026-11-28 闭合**：原 3.35:1 经 `aria-roledescription="gitGraph"` 描边修复后实测 5.04:1（≥4.5），见 `docs/qa/mermaid-dark-2026-11-27.md` §2.2。
2. 实体物理打印机出纸（人工项，register §1.2；保持 NOT RUN）。
3. 系统 DPI 100/125/200 的真实首帧、命中与版心（人工项，register §1.1；保持 NOT RUN）。
4. 真机中文 IME 组合、候选窗、上屏（人工项，register §1.3；保持 NOT RUN）。
5. ~~Narrator/NVDA 语音流程~~：无障碍层已于 `3a6d72d` 移除，该项改判为「非目标」，不再是待验证证据。
6. ~~版本历史面板 5000 快照的阻塞时长~~：已由 Task 5 测量（打开 p95 10.738 ms、滚动 p95 3.642 ms、关闭 p95 0.423 ms），见 `docs/qa/perf-distribution-2026-11-27.md` §1。
7. ~~5000 标题 `Session::open` 的 p95~~：已由 Task 5 测量（n=9，p95 96.463 ms，预算 ≤200 ms），见同上 §1。
8. 全量排版 281 ms 对真实用户"大文档滚动/重排"的体感影响（`docs/review/evidence/perf/current-head-2026-11-26.md:51`、`74-81`，属推断边界）。
9. F-022 的"英文 README 与代码不一致"结论基于源码检索：未检索到 kanban/journey 的深色对比度断言，但存在浅色断言（`lib.rs:2024`、`2050`）；若后续新增未命名断言，需复核该条。

---

**一句话结论**：本轮事实底座登记 34 条（P0 0 / P1 15 / P2 14 / P3 5），其中 13 条由本轮修复或文档收口（Task 2–5 + Task 7.1），5 条已修复，9 条有书面理由接受，7 条因硬件/人工/证据成本延后；历史 27 个含未勾选项的 spec 目录已逐条裁决（仍有效 14 / 已被后续覆盖 6 / 应归档 7），只登记不删除。
