# Phase 5 — 无障碍（Phase gate）

> 评估者：Agent M（独立、只读）｜日期：2026-11-26｜基线：`main` @ `dfa05aa`（Phase 5 提交 `753c962` / `7f0b508` / `dfa05aa`）
> 规格出口：AccessKit 最小语义树 + Narrator 流程 + 限制披露；不得宣称完整屏幕阅读器兼容

## 1. 出口条件

1. `accesskit` / `accesskit_winit` 生产接入，chrome / 菜单 / 模态最小语义树；
2. 菜单打开 → 选择 → 动作、模态打开 → 操作 → 关闭 → 焦点返回可在 Narrator 下完成；
3. 正文语义、高对比度、IME 限制被文档化，不过度宣称。

## 2. 事实

| 项 | 事实 | 证据 |
|---|---|---|
| 语义树 | 窗口、常驻 `MenuBar`、4 个 `Menu`、全部非分隔线 `MenuItem`（UIA 实测 **39** 项）、正文 `MultilineTextInput`（UIA `Edit`，前 8000 字节有界快照）；模态 `Dialog` + 按钮，打开时后台控件移除。 | `../ux/accesskit-uia-2026-11-26.md` §2；`../../accessibility-scope-and-limits.md` §1 |
| UIA 动作 | UIA `InvokePattern` 调「设置…」打开模态，节点数 **46 → 5**；对「取消」Invoke 后回到 **46**；菜单项 Click 走 `App::perform_menu`，禁用项不执行。 | 同上 §3 |
| 自动化 | `cargo test -p veil-ui --features accessibility --lib accessibility` 本次复跑 **9 passed**；无 feature 构建、`veil` crate 测试记录见证据文件。 | 同上 §4；`cargo test … accessibility`（9 passed） |
| 体积 | release exe **14,362,624 B**，门禁 14,680,064 B，余量 **317,440 B**；相对 Task 8 基线增量 **+192,512 B**；SHA-256 已核对为 `1D456A3D…0BD819EC3`。 | `../../component-inventory.md` §13；本机 `Get-Item` / `Get-FileHash` |
| 限制披露 | 正文无 caret / selection / 逐行语义、无 UIA 编辑动作、候选列表未逐项暴露、节点无 bounds、Dialog 映射为 Window、Narrator/NVDA/中文 IME NOT RUN，全部写入文档。 | `../../accessibility-scope-and-limits.md` §2 |
| Narrator 真实语音 | **NOT RUN**：无人值守环境无法可靠驱动并核对语音播报。 | `../ux/accesskit-uia-2026-11-26.md` §6 |

## 3. 判断

**实现项 PASS，出口条件 FAIL。**

- Task 12 的“最小语义树 + 确定性动作路由 + 模态隔离”成立，有 UIA 实测与 9 项单测；体积门禁有余量。
- Task 13 的“限制披露 / 不过度宣称”成立。
- 但规格把「Narrator 流程」写成 Phase 5 出口条件，而 Narrator 与 NVDA **均未执行**；没有真实语音证据，不能判定该出口闭合。按 gate 规则，这不能记为 PASS。

因此结论是：**不得宣称无障碍完成，也不得宣称完整屏幕阅读器兼容**；可实现的部分已成立，未验证的部分保持 NOT RUN。

## 4. NOT RUN

- Narrator 两条流程（菜单浏览并执行「视图 → 专注模式」；设置模态 Tab 到「取消」后关闭、焦点返回正文）；
- NVDA / JAWS 等其他 AT；
- 中文 IME 在无障碍模式下的组字与候选窗；
- 正文 caret / selection / 逐行朗读与 UIA 文本编辑动作（能力未实现，非仅未测）。

## 5. 是否允许进入 Phase 6

**仅允许作为“评估与记录”进入，不允许作为“无障碍完成”收口。** Phase 6 必须在 v2 报告中写明 Narrator / NVDA NOT RUN、正文语义有界、以及“最小支持 + 明确限制”的定位；若要求无障碍出口真正闭合，应回到 Task 13 补真实 Narrator 证据。
