# Phase 0 — 基线与治理

> 生成时间：2026-11-26
> 规格：`.trae/specs/converge-veil-under-jobs-lens/`
> 性质：Phase 0 只冻结事实与执行规则，不改产品代码。

## 1. 基线

| 项 | 值 |
|---|---|
| 分支 | `main` |
| HEAD | `fe3bdd5f09172c7264a040ec7528654dbd38d40a` |
| 版本 | `3.0.2`（`Cargo.toml` workspace） |
| 工作区 | clean（Phase 0 开始时） |
| 工具链 | rustc 1.99.1 / cargo 1.99.1 |
| OS | Windows 11 企业版 10.0.26300 |
| 规格文档 | `.trae/` 被 `.gitignore` 第 28 行忽略，不入库 |

## 2. 证据路径

| 路径 | 用途 | 是否 tracked |
|---|---|---|
| `docs/review/evidence/evaluation/` | Phase gate 评估 | 是 |
| `docs/review/evidence/perf/` | 启动/击键/内存/体积原始样本 | 是 |
| `docs/review/evidence/ux/` | IME/DPI/高对比度/打印/副屏记录 | 是 |
| `docs/review/evidence/spike/` | PDF 断行等 spike | 是 |
| `docs/review/` | v2 报告、组件台账、观察备忘录 | 是 |

## 3. Commit 纪律

- 每个 Task 完成即本地 commit；同一 Task 内独立子任务可各自 commit。
- `git add` 显式文件，不使用 `git add -A`。
- 不 push、不 amend、不 force push、不回滚用户未授权改动。
- `.trae/` 规格文档不入库；证据与报告必须放 tracked 路径。

## 4. 文件所有权

| Agent | 独占写入 |
|---|---|
| Orchestrator | `.trae/specs/converge-veil-under-jobs-lens/*`、`docs/review/evidence/evaluation/phase-0.md` |
| A | `tokens.rs`、`geometry.rs`、`app.rs`、`settings.rs` |
| B | `interaction.rs`、`menu.rs`、`export.rs`、`html.rs`、Mermaid 主题适配 |
| C | `docs/review/evidence/perf/*` |
| D | `docs/review/evidence/ux/*` |
| E | `veil-core` 解析器与测试 |
| F | `config`/`theme` 与测试 |
| G | `view/layout.rs`、`pdf_layout.rs` 测试 |
| H | `docs/review/component-inventory.md` |
| I | `docs/review/evidence/spike/pdf-linebreak.md` |
| J | `docs/review/*` 观察备忘录 |
| K | `app.rs`、`shell.rs`、`Cargo.toml`（Phase 5） |
| M | `docs/review/jobs-lens-report-v2.md`、`docs/review/evidence/evaluation/*` |

同一文件同一时间只允许一个写 agent；A 独占 `app.rs`，B 不写 `app.rs`；K 在 Phase 5 才获得 `app.rs`/`shell.rs` 写权。

## 5. 账本（report / follow-ups → Phase / Task / Agent）

| 来源 | 条目 | 优先级 | Phase / Task | Agent | 验收 |
|---|---|---|---|---|---|
| follow-ups 1.1 | 版心 40 全角仅在 >1600px 生效 | P0 | 1 / 1.1 | A | 常见宽度生效 + 测试 |
| follow-ups 1.2 | 未命名草稿光标不闪 | P0 | 1 / 1.2 | A | 统一闪烁 + 回归 |
| follow-ups 1.3 | `doc_line_height` 只能手改 TOML | P0 | 1 / 1.3 | A | 上屏或标注高级 |
| follow-ups 1.4 | 无统一命令入口；`Ctrl+Shift+P` 是打印 | P0 | 1 / 2.1 | B | 可搜索入口 + 无冲突 |
| follow-ups 1.5 | 远程图片 HTML 导出丢失 | P0 | 1 / 2.2 | B | 内联或可见警告 |
| follow-ups 1.5 | Mermaid 暗色对比 | P0 | 1 / 2.3 | B | 4.5:1 或记录 |
| follow-ups 1.5 | PDF 表格/中文断行差异 | P0 | 1 / 2.4 | B | 修/标注/接受裁决 |
| follow-ups 2.1 | 当前版本性能未复测 | P1 | 2 / 3 | C | 6 项当前 HEAD 证据 |
| follow-ups 2.2 | IME/DPI/高对比度/打印/副屏 | P1 | 2 / 4 | D | 5 项记录 + NOT RUN |
| follow-ups 2.3 | 走查未证据化 | P1 | 2 / 4.6 | D | 可重跑清单 |
| follow-ups 3.1 | 双解析器 | P1 | 3 / 5 | E | 差分 oracle + 分类 |
| follow-ups 3.2 | 双配置解析 | P1 | 3 / 6 | F | round-trip 测试 |
| follow-ups 3.3 | 双排版路径 | P1 | 3 / 7 | G | 跨表面差异清单 |
| follow-ups 3.4 | 组件台账旧基线 | P1 | 3 / 8 | H | 当前 HEAD 重新称重 |
| follow-ups 4.1 | PDF 断行子模块 | P2 | 4 / 9 | I | 六道门 + go/no-go |
| follow-ups 4.2 | `windows` 观察 | P2 | 4 / 10 | J | 观察备忘录 |
| follow-ups 4.3 | `image` 观察 | P2 | 4 / 10 | J | 观察备忘录 |
| follow-ups 4.4 | `callouts → serde_json` 已完成 | P2 | 4 / 11 | H | 记录 |
| follow-ups 4.5 | B 类保留 | P2 | 4 / 11 | H | 记录 |
| follow-ups D1 | 不走纯集成路线 | 决策 | 0 / 账本 | Orchestrator | 已记录 |
| follow-ups D2 | 不以零债务为目标 | 决策 | 0 / 账本 | Orchestrator | 已记录 |
| follow-ups D3 | AccessKit 当前不做 | 决策 | 5 / 12-13 | K | 本 spec 改为 Phase 5 |
| follow-ups D4 | 低成本无障碍基线 | 决策 | 5 / 13.4 | K | 回归 |
| report ⑦ | 适配与上下文感知 3/5 | P1 | 2 / 4 | D | 人体边界证据 |
| report ⑨ | 可扩展与生长 3/5 | P1 | 3 / 5-8、4 / 9-11 | E-H、I-J | 接缝 + 台账 |
| report ③ | 视觉排版 4/5（版心） | P0 | 1 / 1.1 | A | 版心 |
| report ⑤ | 性能 4/5（历史数据） | P1 | 2 / 3 | C | 当前 HEAD |
| report ⑩ | 伦理与控制权 5/5 | 保持 | 5 / 13.3 | K | 不越界、不过度宣称 |

## 6. Phase 0 出口

- [x] 基线记录可被后续 Phase 引用
- [x] 账本覆盖 report 与 follow-ups 的全部可执行条目
- [x] 证据路径与 tracked 状态明确
- [x] commit 纪律与文件所有权明确
- [x] 无产品代码改动

## 7. 风险与例外

- Phase 2 的交互类验证（IME、Narrator、物理打印）若当前环境无法真实执行，必须在证据文件中标注 NOT RUN，不得写成通过。
- Phase 5 AccessKit 复用既有 Task 3 Go 结论；若生产接入成本超出本阶段预算，允许在 Phase 5 内缩小到“chrome/菜单/模态最小树”，但不得提前到 Phase 1-4。
- `.trae/` 规格文档不进入 git；v2 报告放 `docs/review/`，确保可追踪。
