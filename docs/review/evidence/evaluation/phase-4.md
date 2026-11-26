# Phase 4 — 选择性集成（Phase gate）

> 评估者：Agent M（独立、只读）｜日期：2026-11-26｜基线：`main` @ `dfa05aa`（Phase 4 提交 `ff77aea` / `4a22306` / `271372c`）
> 规格出口：PDF 断行子模块 go/no-go、`windows` / `image` 观察备忘录、B 类保留与 `callouts` 记录

## 1. 出口条件

1. PDF 断行候选有当前实现基线与候选对比，六道门逐项有结论；
2. `windows` / `image` 只记录收益、体积、行为差异、触发条件，不改生产依赖；
3. 台账回写 B 类保留理由、`callouts → serde_json` 完成状态，并追加 Task 9/10 结论。

## 2. 事实

| 项 | 事实 | 证据 |
|---|---|---|
| PDF 断行 spike | **NO-GO**。8 组语料 118 行中 7 组断点逐行一致；仅 `mixed` 2 行差异（当前止于词边界，候选挤满 CJK 边界），候选无质量净收益。候选只接受 `&str`，丢失 `LItem` 与 `last_space` 偏好，行为契约不通过；`icu_segmenter` 数据 crate 压缩 3,141,236 B，超当前 509,952 B 余量，不引入。生产代码未改，仅 dev-dependency 与 2 个测试；本次复跑 2 项 spike 测试通过。 | `../spike/pdf-linebreak.md` §3/§6/§7；`cargo test -p veil --lib pdf_linebreak_spike`（2 passed）；`../../component-inventory.md` §12.1 |
| `windows` / `windows-registry` | 生产依赖图**没有** `windows` 包；手写 79 条 `extern "system"`、16 个 COM 槽位（`windows.rs` 9 + `wic.rs` 7）。决策：不引入，只观察；触发条件 5 条（FFI 增长、可归因 UB、成批新 API、AccessKit 带入 `windows 0.62`、需要 ABI 审计证据）。 | `../../jobs-lens-observation-windows-image.md` §2.1/§3；台账 §12.2 |
| `image` | 订单订正：PNG 走 `png` 0.18，非 PNG 走系统 WIC；`image` 0.25.10 已由 `arboard` 传递引入但只启用 `bmp` + `png`。决策：不引入新 feature，只观察；触发条件 4 条。 | 同上 §2.3/§4；台账 §12.2 |
| B 类与 callouts | 8 项 B 类保留理由（`quick_open`、`html2md`、`theme` 读取、`i18n`、字面搜索、`link_safety`、`slug`、`cli`）已落账；`callouts → serde_json` 记为已完成（B-1 Done），保留 `validate` 薄封装。 | `../../component-inventory.md` §12.3 |

## 3. 判断

**PASS（交付物门禁）。** go/no-go 有数据与六道门结论，观察项无生产改动且有“何时再评估”，台账追加了 B 类与 callouts 记录，四项均与 spec 的“先测后换 / 只观察 / 不整体替换”一致。

流程瑕疵（不影响技术结论）：`tasks.md` 的 Task 9/10/11 与 `checklist.md` 的 Phase 4 复选框仍未勾选，而证据与台账回写已完成。这属于 orchestrator 的记账滞后；本评估只记录，不修改规格文件。

## 4. NOT RUN / 已知缺口

- `unicode-linebreak` 的质量结论只覆盖策略层（`&str` + `FakeMetrics`），未做真机字体、链接跨行、图片盒的端到端替换测量；接入后的体积也未实测（当前仅 dev-dependency）。
- `windows` / `image` 的体积增量、编译时间、逐像素对拍均为**推断**，未在本阶段称重。
- PDF 侧 `—` / `～` 行首禁则缺口不因换候选而消失，仍未裁决。

## 5. 是否允许进入 Phase 5

**是。** 选择性集成有明确 NO-GO 与观察触发条件，无生产依赖变更；无障碍可以按计划在 Phase 5 串行开始。
