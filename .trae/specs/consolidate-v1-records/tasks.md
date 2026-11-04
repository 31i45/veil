# Tasks

- [x] Task 1: 统一走查台账状态口径（`docs/review/3e-walkthrough-results.md`）
  - [x] SubTask 1.1: 盘点 C/D/E/F/G/H 全部条目，把「剩余人工项状态」「真机走查轮次 1」两张表的重复结论合并为唯一状态（已通过 / 已修复 / 未解决 / 未测）
  - [x] SubTask 1.2: 每项补证据：自动化项写测试名，修复项写 commit hash（本轮为 `4deeca9`），未解决项写截图/现象
  - [x] SubTask 1.3: 删除或改写过期结论（D12「已修复」、C11 旧素材说明），保证表内无互斥状态

- [x] Task 2: 补齐未处理问题清单（`3e-walkthrough-results.md` 的「遗留未处理项」表）
  - [x] SubTask 2.1: D12-b：现象（`**C|hi*n*ese**` 形态）、已知结论（编辑路径正确）、嫌疑（reveal 按块展开 / 块级 inline 解析 / 实际源码形态）、归属（用户自行处理）
  - [x] SubTask 2.2: C5 表格、C2 行首禁则、C7 图片细测：补「现象 / 下一步 / 归属 v1.1」
  - [x] SubTask 2.3: 内核隐患三条（`math_source` 长度守卫、`build_refs` 全量重建、`bracket_pairs` 跨强调边界）：标明归属批次 3 或 v2

- [x] Task 3: 版本规划归位并对齐 Gate Decision
  - [x] SubTask 3.1: 核对 `3e-walkthrough-results.md` 的「未来版本规划候选」与 `.trae/specs/v1-progress-review-and-roadmap/review.md` 的 Gate Decision，消除重复/冲突表述
  - [x] SubTask 3.2: 四档标注来源与触发条件：v1.0 门禁（spec）、v1.1 收尾（走查遗留）、批次 3（`remediation_plan_v2.md` 六项）、v2 候选（ADR-010/ADR-011 等）
  - [x] SubTask 3.3: 在 `3e-manual-walkthrough.md` 同步 D12 未解决标注与索引

- [x] Task 4: 同步项目记忆 `project_memory.md`
  - [x] SubTask 4.1: 更新进度快照：`cargo test --workspace` 894 passed / 12 ignored、release 5,840,896 B、commit 链至 `4deeca9`
  - [x] SubTask 4.2: 保留并确认 2 条本轮 lessons（marker toggle 正确层次、编辑路径≠功能正确的排查顺序）
  - [x] SubTask 4.3: 「已知未处理问题」与文档清单一一对应（D12-b / C5 / C2 / C7 / 未测区 / 内核三条）

- [x] Task 5: 一致性校验（只读检查，不改代码）
  - [x] SubTask 5.1: 全库检索 `D12`、`C5`、`C2`、`C7`、`已修复`，确认无过期结论残留（历史章节均加「历史快照」标注；已完成 spec 档案保留原样）
  - [x] SubTask 5.2: 校验文档内引用的测试名/commit 真实存在（`cargo test` 名称检索 + `git log`）
  - [x] SubTask 5.3: `git status` 确认变更仅限文档与 `.trae/specs`；无任何 `crates/**` 改动

# Task Dependencies

- Task 2 依赖 Task 1（状态口径统一后再写未处理清单，避免重复）
- Task 3 依赖 Task 2（规划档位引用未处理项归属）
- Task 4 依赖 Task 1–3（记忆快照引用最终文档结论）
- Task 5 依赖 Task 1–4
