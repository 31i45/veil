# Phase 1 — P0 用户体验（Phase gate）

> 评估者：Agent M（独立、只读）｜日期：2026-11-26｜基线：`main` @ `dfa05aa`（Phase 1 提交 `c3541e1`…`5b3d9a6`）
> 规格出口：5 项 P0 均有验收或明确裁决（`.trae/specs/converge-veil-under-jobs-lens/spec.md` Phase Plan）

## 1. 出口条件

1. 版心在常见窗口生效，窄窗降级为可用宽度；
2. 未命名草稿与已保存文档光标闪烁一致，首帧可见；
3. 行高可在设置界面调整，或明确标注为高级 TOML；
4. 存在可搜索、键盘可达的统一命令入口，且不与打印冲突；
5. 远程图片 / Mermaid 暗色 / PDF 表格与中文断行逐项有“修 / 标注 / 接受”裁决。

## 2. 事实

| 项 | 事实 | 证据 |
|---|---|---|
| 1.1 版心 | `content_span` 对除 `Full` 外的档位一律封顶 `READABLE_COLS(40) × 16px × scale`；可用宽不足时取可用宽、不裁切；`WIDE_WINDOW` 门槛已删除。本次复跑 3 项几何测试全过（640 / 1200 / 1920 三档）。 | `crates/veil-ui/src/geometry.rs:116-142`；`theme.rs:557-560`；`tokens.rs:344`；`cargo test -p veil-ui --lib content_span`（3 passed） |
| 1.2 光标 | `blink_active()` 只依赖打印态与 IME preedit，不再判断 `doc.path`；未命名草稿有专门回归测试。本次复跑 5 项 blink 测试全过。 | `crates/veil-ui/src/app.rs:1650-1656`；`app_tests/chrome_ui.rs:604-654`；`cargo test -p veil-ui --lib blink`（5 passed） |
| 1.3 行高 | 设置表单含「正文行高」数值字段（10..=96，整数步进，apply 时夹到 `[字号, 3×字号]`）；设置 UI 不再是隐藏能力。本次复跑 1 项测试通过。 | `crates/veil-ui/src/view/settings.rs:243-254/576-581`；`cargo test -p veil-ui --lib doc_line_height`（1 passed） |
| 1.4 命令入口 | `Ctrl+Shift+P` 打开命令面板，`Ctrl+P` 保持快速打开，打印改为 `Ctrl+Alt+P`；候选来自全部非分隔线菜单项（UIA 实测 39 项）。本次复跑 3 项测试通过，含中文子串过滤并执行「打字机模式」「设置」。已知取舍：AltGr 布局下组合键可能与字符输入冲突，已在源码注明。 | `crates/veil-ui/src/interaction.rs:2000-2028`；`view/quick_open.rs:190-205`；`input.rs:184`；`app_tests/chrome_ui.rs:1378-1452`；`cargo test -p veil-ui --lib command_palette`（3 passed） |
| 1.5 跨表面 | 远程图片：HTML 导出仍删除远程 `src`（离线契约），但出现可见中英双语警告块，覆盖 Markdown 图片与行内 `<img>`；Mermaid 暗色：接受 + 文档化，7 项暗色对比测试通过；PDF 表格 / 中文断行：接受 + 文档化，并移交 Task 9（结论 NO-GO，保留自研断行）。本次复跑远程图片警告测试通过。 | `docs/review/jobs-lens-cross-surface.md` §1–§3；`crates/veil/src/export.rs`；`cargo test -p veil --lib remote_markdown_image_adds_visible_export_warning`（1 passed）；`evidence/spike/pdf-linebreak.md` |

## 3. 判断

**PASS。** 5 项出口条件均满足：版心与光标有代码 + 回归测试，行高已上屏，命令入口有测试且与打印无冲突，跨表面三项均有用户可见裁决或警告；没有发现“静默丢图”或“只能猜配置文件”的残留路径。

限制与保留：

- Mermaid 暗色只覆盖 flowchart / mindmap / gantt / c4 stereotype 的断言；其余 16 种图型暗色观感为 **NOT RUN**，c4 `Rel` 连线标签无断言（已在裁决文档披露）。
- PDF / 打印中的远程图片仍降级为 alt / `[图片]`，只由 README 披露，不在 PDF 内加警告；这是“接受 + 标注”，不是“已修复”。
- 版心只由布局测试覆盖窄 / 常见 / 超宽三档，未重新采集真机截图；按 checklist“测试或截图”口径可接受。

## 4. NOT RUN

- 16 种 Mermaid 图型暗色逐图人工目检；
- 物理打印中的远程图片降级观感；
- 真实用户对命令面板的发现性验证（无用户测试）。

## 5. 是否允许进入 Phase 2

**是。** P0 无未裁决项；上述 NOT RUN 已在文档中标记，不冒充通过。
