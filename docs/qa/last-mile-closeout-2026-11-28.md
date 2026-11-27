# 最后一公里收口（2026-11-28）

> 规格：`.trae/specs/close-last-mile-and-ignored-gates/`（spec / tasks / checklist）。
> 基线：`main` HEAD `85b0953`（Task 9），版本 `3.0.2`；Task 7–9 已提交，Task 10 的文档收口轮只改文档、未提交、未推送。
> 本文件逐条给出 10 个任务的最终状态、证据位置、仍未完成的人工项，以及用户需要执行的最小命令。
> 纪律：没有真实执行或真实输出的项一律写 **NOT RUN** / **PENDING**，不写成 PASS。

## 0. 一句话结论

- 可自动化的部分全部闭合：18 条 `#[ignore]` 探针由一条 release 门禁覆盖（本轮 **PASS，cargo exit 0，FAIL 0**），4 条代码差距（容器内 `<details>`、超高显示公式、Mermaid 加载态与就地允许远程、gitGraph 对比度）已修复并有测试锁定。
- 仍需人在场的 3 项（DPI 100/125/200、实体打印机出纸、真机 IME）保持 **NOT RUN**，但已做成 assisted 流程：用户只做物理动作，脚本负责截图、落盘与判定表。
- 直接修复轮（Task 7–10）已闭合：Callout 内 `<details>` 与普通引用统一提升（`2060da9`）；10 MB 单块首键 p95 **976.024 → 147.227 ms**（`67cdd8d`，预算 200 ms 写进测试）；ignored 门禁简化为探针自证（`85b0953`，`probe-gate.ps1` 694 → 153 行）。
- 无「未处理且未解释」项：每项要么有测试/证据，要么有明确的人工命令与未执行原因。

## 1. 十个任务的最终状态

| 任务 | 状态 | 证据位置 | commit |
|---|---|---|---|
| Task 1 ignored 清单与 release 探针门禁 | **完成**（基线 JSON 已由 Task 9 删除） | `docs/qa/ignored-tests-2026-11-27.md`；`tools/verify/probe-gate.ps1`；`docs/qa/probe-gate-2026-11-28.md`；README 中英开发者章节 | `68846e9` |
| Task 2 容器内 `<details>` 识别 | **完成** | `crates/veil-core/src/syntax/block.rs`；`crates/veil/tests/syntax_matrix.rs`（ADJX/ADJC 期望）；`crates/veil-ui/src/app_tests/blocks.rs`；`docs/qa/report-2026-11-27.md` | `370f522` |
| Task 3 超高显示公式打印一致性 | **完成** | `crates/veil-ui/src/print.rs`；`crates/veil/src/platform/pdf_layout.rs`；单测 `print_scales_tall_display_math_to_one_page`、`tall_display_math_scales_to_single_page_content_height`；`docs/qa/perfect-surface-audit-2026-11-27.md` F-009 | `f89bb99` + `906934c`（规则文档更正） |
| Task 4 Mermaid 视图态与 gitGraph 对比度 | **完成** | `crates/veil-ui/src/paint.rs`（「渲染中…」胶囊）；`crates/veil-ui/src/view/ctx_menu.rs`（就地允许远程）；`crates/veil-mermaid-offline/src/lib.rs`（gitGraph 描边）；`docs/qa/mermaid-dark-2026-11-27.md`；证据 `docs/qa/evidence/mermaid-dark-2026-11-27/gitgraph-dark.{png,svg}` | `66fe135` |
| Task 5 人机协同与被动手册 | **自动化部分完成；人工项 NOT RUN** | `tools/verify/assisted/{dpi,print,ime}.ps1` + `README.md`；`docs/qa/perf-distribution-2026-11-27.md`；证据 `docs/qa/evidence/assisted-2026-11-27/` | `d57be44` |
| Task 6 文档、门禁与收口 | **完成** | 本文件；`docs/qa/release-facts-2026-11-27.md`「最后一公里与 ignored 门禁（2026-11-28）」；README 中英已知限制/开发验证；`docs/qa/perfect-surface-audit-2026-11-27.md` | `3ee2dcd` |
| Task 7 Callout 内 `<details>` 与普通引用同规则 | **完成** | `crates/veil-core/src/syntax/block.rs`；`crates/veil/tests/syntax_matrix.rs`（`ADJC2-061/083`）；`docs/qa/golden/html/CASE-ADJC2-{061,083}.html`；`docs/qa/adjacency-digest.txt` | `2060da9` |
| Task 8 10 MB 单块首键延迟 | **完成** | `crates/veil-ui/src/app_tests/render.rs`（`perf_huge_block` 内断言 70 / 80 / 200 ms）；`docs/qa/perf-distribution-2026-11-27.md` §1「修复后复测」；`docs/qa/probe-gate-2026-11-28.md` | `67cdd8d` |
| Task 9 简化 ignored 门禁 | **完成** | `tools/verify/probe-gate.ps1`（694 → 153 行）；`docs/qa/ignored-tests-2026-11-27.md`；`docs/qa/perf-distribution-2026-11-27.md` §3；删除 `perf-baseline.json`、`perf-baseline-final.json` 与 3 份 stability 归档 | `85b0953` |
| Task 10 文档与门禁收口 | **完成**（本轮文档收口，未提交） | 本文件；`docs/qa/release-facts-2026-11-27.md`「直接修复轮（2026-11-28）」；README 中英；`docs/qa/perf-distribution-2026-11-27.md`、`docs/qa/probe-gate-2026-11-28.md` | 本轮未提交 |

## 2. 逐任务要点与复核

### Task 1：ignored 可见且可回归

- 源码 `#[ignore]` 真实计数 **18**（`rg -n "#\[ignore" crates --glob '*.rs'`）：13 条性能探针、4 条病态/超大单块探针、1 条 CJK PDF 证据生成器；另有 1 条诊断器 `mindmap_viewbox_debug` 已晋级为常规断言。
- `pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 9`：**判定行 20，PASS 20 / FAIL 0 / SKIP 0**，总耗时 **399.2 s**（`pathological_eval_child` 以 loop / microtask 各判定一次）。（历史口径；`-Repeat` 与判定层已由 Task 9 简化。）
- 门禁按指标类型选择 p50/p95 并叠加测量噪声地板；产品预算硬性优先，超预算直接 FAIL，未放宽任何探针断言。（已被 Task 9 取代：判定下沉到测试内自证上限，无外部基线、无噪声地板。）
- 三次连续稳定性：`docs/qa/evidence/assisted-2026-11-27/probe-gate-stability-{1,2,3}.md`，均 PASS 20 / FAIL 0 / SKIP 0。（归档已随 `85b0953` 删除，此处仅保留历史记录。）
- 如实记录：稳定性验证过程中曾有一次 `perf_after_edit_breakdown` `peak_total_ms` FAIL（p50 5.96 ms vs 阈值 5.613 ms）；单探针 9 次复测 p50 3.812 ms，随后三次连续 PASS。该次保留为宿主瞬时毛刺，未改写成 PASS。

### Task 2：容器内 `<details>`

- ListItem / BlockQuote 的容器展开路径现在把 `<details>` 提升为独立 Details 块，并保留前缀与内层块归属。
- 回归覆盖：`details_inside_list_item_becomes_details_block`（core）、`quote_container_details_collapse_toggles_by_summary_click`、`list_item_container_details_collapse_toggles`、`promoted_container_details_elide_quote_markers_in_wysiwyg`（UI）；`ADJX-003/006` 金样与 `adjacency-digest.txt` 已更新，不再锁定降级形态。
- `cargo test --workspace` 中 `syntax_matrix` 5 项全绿（2148 用例 × 7 表面，311.30 s）。

### Task 3：超高显示公式

- 打印与 PDF 对高于一页的显示公式采用同一规则：等比缩放到一页可用高内（打印页帧高 × 80%，PDF 内容高 − 12pt），块高/绘制/命中同口径；不丢内容、不跨页硬切。
- 单测 `print_scales_tall_display_math_to_one_page`、`tall_display_math_scales_to_single_page_content_height` 随本轮 workspace 门禁通过。
- README 中英已从「已知限制」移除该差异，规则并入 PDF/打印表格条目；台账 F-009 标注 2026-11-28 复核通过。

### Task 4：Mermaid 与 gitGraph

- 加载态：Mermaid 块在「已排队 / 渲染中」时显示块右上角「渲染中…」胶囊，回包后自动消失；失败/空/远程被阻止仍可查看源码或重试。测试 `mermaid_pending_block_shows_loading_hint_until_bitmap_arrives`。
- 远程图片：被全局策略阻止时，右键菜单就地提供「允许远程图片（全局）…」，确认后立即重排重试，无需进入设置页。测试 `remote_blocked_image_menu_allows_in_place_and_retries`、`reload_blocked_remote_image_points_to_allow_remote_images`。
- gitGraph 分支标签：仅按 `aria-roledescription="gitGraph"` 给标签文字加细描边（不改分支配色/药丸底色），实测由 3.35:1 提升到 **5.04:1**（≥4.5:1）；测试 `dark_gitgraph_branch_label_meets_wcag_aa`。README 中英旧值已更新。

### Task 5：人机协同与性能分布

- assisted 脚本一条命令启动、分步提示、自动截图/落盘、生成判定表；用户只做物理动作。说明见 `tools/verify/assisted/README.md`。
- 性能分布：5000 标题打开 p95 **96.463 ms**、滚动 p95 **1.098 ms**；版本历史面板 5000 快照打开 p95 **10.738 ms**、滚动 p95 **3.642 ms**、关闭 p95 **0.423 ms**（n=9，详见 `docs/qa/perf-distribution-2026-11-27.md`）。
- 人工项状态见 §3，保持 NOT RUN / PENDING。

### Task 6：文档、门禁与收口

- README 中英「已知限制 / 开发与验证」：移除已闭合的容器内 details、超高公式打印差异、Mermaid 无加载态、无就地允许远程、gitGraph 3.35:1 旧值；人工项改为指向 assisted 脚本并写清「用户只做物理动作」；ignored 计数更新为 18；probe-gate 判定口径改为按指标类型选择 p50/p95。
- `docs/qa/release-facts-2026-11-27.md` 新增「最后一公里与 ignored 门禁（2026-11-28）」：测试数、ignored 清单、探针门禁、性能分布引用、体积、SHA、依赖零变化、未推送、版本号与 commit 列表。
- `docs/qa/perfect-surface-audit-2026-11-27.md`：F-004（容器内 details）与 F-009（超高公式）标为已闭合；F-015 / F-016 由 Task 5 测量闭合；§4 收口清单对应条已勾选；§5 残留说明已更正（gitGraph 3.35:1 → 5.04:1，历史面板与 5000 标题 p95 已测）。

### Task 7：Callout 内 `<details>` 与普通引用同规则

- 删除 `block.rs` 中 `(!is_callout).then_some(depth)` 的按容器特判，Callout 与普通 BlockQuote 走同一提升路径；前缀与内层块归属不变。
- `ADJC2-061/083` 金样与 `docs/qa/adjacency-digest.txt`、`docs/qa/report-2026-11-27.md` 同步更新；Callout 标题只出现一次。`syntax_matrix` 5 项随本轮全量通过。

### Task 8：10 MB 单块首键延迟

- 归因：主因是每次击键把整块 10 MB 重新 inline 解析（约占 82%）；改为只解析需渲染前缀、把光栅限制在可见行带。
- 实测：10 MB 首键 p95 **976.024 → 147.227 ms**、文末插入 975.732 → **153.297 ms**；1 MB 首键 160.465 → 46.436 ms（本轮 `probe-gate` 真实输出）。
- 预算写进 `perf_huge_block`：1 / 2 / 10 MB = **70 / 80 / 200 ms**（regression ceiling），既有「前缀降级 / 不超上限 / 原文往返」断言保留。

### Task 9：简化 ignored 门禁

- 判定下沉到测试自证：每条性能探针在测试内断言回归上限，单次观测采 3 次取 p50，归因/诊断探针断言结构不变量。
- `probe-gate.ps1` 由 **694 行降到 153 行**：只做 release 构建 + 一次 `cargo test --release -- --ignored` + 归档；删除 `perf-baseline.json`、`perf-baseline-final.json` 与 3 份 stability 归档。
- 本轮门禁一次通过：PASS / FAIL 0，构建 2.2 s、运行 38.4 s，归档 `docs/qa/probe-gate-2026-11-28.md`。

### Task 10：文档与门禁收口

- `docs/qa/release-facts-2026-11-27.md` 新增「直接修复轮（2026-11-28）」：2269 / 0 / 18、probe-gate PASS、10 MB 前后、Callout details、14,312,448 B、SHA、commit 列表、未推送。
- 本文件补齐 Task 7–10 状态与证据；README 中英核对「Callout 内 details 不提升」「10 MB 首键 ~1 s」「ignored 需基线 JSON」三类旧描述均已不存在，已知限制只剩真实未闭合项。
- `docs/qa/perf-distribution-2026-11-27.md` 与 `docs/qa/probe-gate-2026-11-28.md` 措辞已与新机制一致（无外部基线 / 噪声地板）。

## 3. 仍未完成项（人工，NOT RUN）

| 项 | 状态 | 缺哪一步 | 证据文件 |
|---|---|---|---|
| 系统 DPI 100% / 125% / 200% | **NOT RUN**（150% 已抓图 PASS） | 在 Windows 设置切换缩放并重新登录/重启应用后重跑脚本 | `docs/qa/evidence/assisted-2026-11-27/dpi.jsonl`（100/125/200 各一行 `status=NOT_RUN`）；`dpi-150.png` |
| 实体打印机出纸 | **NOT RUN**（六项 PENDING） | 装 A4 纸 → 应用内 `Ctrl+Alt+P` 选实体打印机 → 逐项核对出纸 | `docs/qa/evidence/assisted-2026-11-27/print-checklist.md`、`print-sample.md` |
| 真机中文 IME（微软拼音 / 微软五笔） | **NOT RUN** | 切到微软拼音（五笔再跑一遍），按提示完成六步并回车 | `docs/qa/evidence/assisted-2026-11-27/ime-result.md` |
| 远端 GitHub Actions 与 Linux 桌面 | **NOT RUN** | 推送/触发远端 CI（本规格明确不推送，故不做） | README 已知限制 |
| 真实断网抓包归因 | **NOT RUN** | 在真实断网环境抓包复验 | README / `PRIVACY.md` |
| 无 CJK 字体的系统与只读导出目标 | **NOT RUN** | 在无 CJK 字体机器与只读目录复验 | README 已知限制 |

> 后三项不属于本规格的「最后一公里」代码/门禁范围，沿用既有登记；不因本轮文档更新改变其 NOT RUN 状态。

## 4. 用户只需执行的一条命令（逐项）

三个物理项各自一条命令；不存在一条命令能在不切换系统设置的前提下覆盖全部三项，因此不伪造这种入口：

```powershell
# DPI：先在「设置 → 系统 → 屏幕 → 缩放」切到目标档位并重启 veil.exe，再执行
pwsh -NoProfile -File tools/verify/assisted/dpi.ps1 -Dpi 100 -Capture   # 125 / 200 同理

# 实体打印机：装纸后执行，再按 Ctrl+Alt+P 打印并核对出纸
pwsh -NoProfile -File tools/verify/assisted/print.ps1

# 真机 IME：切到微软拼音后执行，按提示完成六步
pwsh -NoProfile -File tools/verify/assisted/ime.ps1
```

若只想先补一项，最小闭环是实体打印（一条命令即可生成样张、驱动清单与核对表并打开应用）：

```powershell
pwsh -NoProfile -File tools/verify/assisted/print.ps1
```

## 5. 门禁快照（真实输出）

直接修复轮（2026-11-28 07:21–07:33）：

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace` | **2269 passed / 0 failed / 18 ignored**（分套件汇总 2269；`veil-mermaid-offline` 子进程两次复跑同一二进制，输出另各 1 passed） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings（exit 0） |
| `cargo fmt --all --check` | 通过（exit 0） |
| `pwsh -NoProfile -File tools/verify/probe-gate.ps1` | **PASS / FAIL 0**（构建 2.2 s、运行 38.4 s），一次通过 |
| `cargo build --release -p veil` | `target/release/veil.exe` = **14,312,448 B**（余量 367,616 B） |
| SHA-256 | `3B567E860AE2337393261502079140DEC6F316151F0B6236719165AED57928FC` |
| 10 MB 单块首键 p95 | **147.227 ms**（文末插入 153.297 ms；修复前 976.024 / 975.732 ms） |
| 推送 / tag / 版本 / 依赖 | 未推送、未打 tag、版本仍 3.0.2、`Cargo.toml` / `Cargo.lock` 自 `3ee2dcd` 起无改动 |

Task 6 轮次旧值（已被本表取代）：2269 / 0 / 18（套件汇总 2267）、`-Repeat 9` PASS 20、14,309,888 B、SHA `7B5DD8…BC6E`。

## 6. 收口声明

- 本规格 10 个任务均有最终状态与证据位置；没有「未处理且未解释」项。
- 所有 PASS 均来自本轮真实命令输出；NOT RUN / PENDING 保持原状，未改写成 PASS；本轮门禁一次通过，无偶发失败。
- Task 7–9 的源码改动已提交（`2060da9` / `67cdd8d` / `85b0953`）；Task 10 收口只改文档，未修改源码/测试，未删除断言，未推送 GitHub，未执行 `git commit`。
