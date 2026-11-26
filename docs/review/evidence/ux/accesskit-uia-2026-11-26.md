# AccessKit / UIA 无障碍证据（Task 12）

> 日期：2026-11-26
> 基线 HEAD：`271372c`（Phase 1-4 完成）+ 本次 `feat(a11y)` 提交
> 工具链：rustc/cargo 1.99.1；Windows 11 企业版 10.0.26300；系统 DPI 144（150%）
> 构建：`cargo build --release -p veil`（默认 feature，含 `accessibility`）

## 1. 语义树结构（设计事实）

| 层级 | Role | Node ID | 说明 |
|---|---|---|---|
| 窗口 | `Window` | 1 | label = 窗口标题 |
| 菜单栏 | `MenuBar` | 100 | **常驻**，不依赖菜单是否打开 |
| 文件 / 编辑 / 视图 / 帮助 | `Menu` | 200/300/400/500 | 每个菜单挂全部非 Separator 项 |
| 菜单项 | `MenuItem` | 基址+1+行号 | label、keyboard_shortcut、disabled、`Click`+`Focus` |
| 正文 | `MultilineTextInput`（UIA: `Edit`） | 2 | label「正文」；value = 前 8000 字节有界快照 |
| 设置/模态 | `Dialog` + `Button` | 10000、10001+ | modal；打开时后台控件从树中移除 |
| 查找 / 大纲 / 快速打开 / 右键菜单 | `Group` / `Menu` | 32000 / 30000 / 31000 / 20000 | 保持旧角色；快速打开与命令面板 name 不同 |

动作路由：菜单项 `Click` → 关闭菜单后调用现有 `App::perform_menu`（禁用项不执行）；模态按钮 `Click` →
现有 `resolve_modal` / `resolve_settings`；`Focus` 只写入焦点覆盖，不伪造视觉状态；模态打开时后台命令拒绝。

## 2. 真实 UIA 取证 — PASS（菜单项与正文可见）

脚本：`tools/verify/ac11_uia_dump.ps1 -Scenario menu -WaitMs 3000`

```text
节点总数（含根）：46，除根节点外：45
MenuItem 数量：39
ControlType.Window | "veil"
  ControlType.MenuBar | "主菜单栏"
    ControlType.Menu | "文件"
      ControlType.MenuItem | "新建"
      ...
      ControlType.MenuItem | "导出 PDF…"
      ControlType.MenuItem | "打印…"
    ControlType.Menu | "编辑"
      ControlType.MenuItem | "撤销" | False   ← 禁用态经 UIA 可见
      ControlType.MenuItem | "重做" | False
      ...
    ControlType.Menu | "视图"
    ControlType.Menu | "帮助"
  ControlType.Edit | "正文"
```

结论：UIA 无需展开菜单即可看到四个顶层菜单全部 39 个菜单项，以及 `正文` 文本控件；
这直接覆盖了旧实现“Narrator 只能读窗口标题”的失败点。

## 3. 真实 UIA 动作路由 — PASS（UIA InvokePattern）

在临时 PowerShell 取证脚本中，对 UIA 树里的菜单项 `设置…` 调用 `InvokePattern`：

```text
ControlType.Window | "veil"
  ControlType.Window | "设置…"        ← AccessKit Dialog 在 UIA 下映射为 Window
    ControlType.Button | "确定"
    ControlType.Button | "恢复默认"
    ControlType.Button | "取消"
```

- 模态打开后节点数从 46 → 5：菜单栏、正文、面板全部从树中移除；
- 再对 `取消` 调用 `InvokePattern`，节点数回到 46，菜单栏与正文恢复；
- 证明菜单项 Click 与模态按钮 Click 均走通现有产品路径，而非只记录日志。

## 4. 自动化测试

| 命令 | 结果 |
|---|---|
| `cargo test -p veil-ui --lib`（无 feature） | 1434 passed, 0 failed, 12 ignored |
| `cargo test -p veil-ui --lib --features accessibility` | 1443 passed, 0 failed, 12 ignored（含 9 项 a11y） |
| `cargo test -p veil`（默认 feature） | 253 + 6 + 4 passed, 0 failed |
| `cargo test -p veil --features accessibility` | 253 + 6 + 4 passed, 0 failed |

a11y 单测覆盖：四菜单常驻与全部菜单项、正文节点、disabled 与 `action_enabled` 同源、
模态移除后台控件、菜单 Click → `MenuAction`、禁用项不执行、模态按钮 `resolve_modal`、
模态打开拒绝后台命令、Focus 覆盖与 stale id 回退、快速打开/命令面板 name 区分。

## 5. 体积门禁 — PASS

| 项 | 值 |
|---|---|
| 新 release exe | **14,362,624 B** |
| 14 MiB 门禁 | 14,680,064 B |
| 余量 | 317,440 B |
| 相对基线 14,170,112 B 增量 | +192,512 B（约 188 KiB） |
| SHA-256 | `1d456a3d532cec10a8d141592281d502e88a419bfcc22ec3cb8cb0d0bd819ec3` |

## 6. Narrator 真实语音流程 — NOT RUN

- 未执行：无人值守环境无法可靠驱动 Narrator 语音并核对其播报内容。
- 不冒充通过；本轮可复现证据止于 UIA 树与 UIA 动作调用（上两节）。
- 待人工复核流程：① Narrator 下浏览菜单并执行「视图 → 专注模式」；② 设置模态内
  Tab 到「取消」后关闭，焦点返回正文。

## 7. 已知限制（不得过度宣称）

- 正文仅暴露**有界纯文本快照**（前 8000 字节）；caret、selection、逐行文本与文本编辑动作
  尚未映射，Narrator 不能按字符/行精读全文。
- 语义节点未提供屏幕边界，UIA 树中多数节点 `BoundingRectangle` 为 `empty`；
  屏幕阅读器可按名称与角色导航，但触摸探索/命中定位不完整。
- AccessKit `Dialog` 在 Windows UIA 下呈现为 `ControlType.Window`，而非 `ControlType.Pane/Dialog`。
- 快速打开/查找/大纲仅暴露分组名称，候选列表尚未逐项暴露；右键菜单只暴露菜单节点。
- 不宣称完整屏幕阅读器兼容；本证据只证明菜单项、正文入口、模态与按钮动作可达。
