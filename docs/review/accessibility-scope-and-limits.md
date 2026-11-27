# Veil 无障碍支持范围与限制（AccessKit / UIA）

> **已归档（2026-11-27）**：AccessKit/UIA 层已于提交 `3a6d72d`（`refactor(ui)!: 永久移除无障碍（AccessKit）能力`）永久移除。本文描述的 `accessibility` feature、UIA 语义树与构建命令**均已不存在**，仅作历史记录；当前产品不提供屏幕阅读器支持，见 `README.md` / `README.zh-CN.md` 的「无障碍」章节。

> 日期：2026-11-26
> 适用版本：`feat(a11y)` 提交 `753c962` 起的发行产物（默认 feature 含 `accessibility`）
> 关联证据：[accesskit-uia-2026-11-26.md](evidence/ux/accesskit-uia-2026-11-26.md)、[current-head-2026-11-26.md](evidence/ux/current-head-2026-11-26.md)

## 结论（一句话）

Veil **只宣称**：Windows UIA 能导航到常驻菜单项与正文入口，且菜单项/模态按钮的 UIA Click 能走通现有产品路径。
Veil **不宣称完整屏幕阅读器兼容**，也不宣称 Narrator/NVDA 的语音流程已验证。

## 1. 已支持（有自动测试或真实 UIA 取证）

| 能力 | 语义树暴露 | 证据 |
|---|---|---|
| 窗口 | `Window`，label = 窗口标题 | `cargo test -p veil-ui --features accessibility` a11y 单测 |
| 菜单栏 | `MenuBar` 常驻，不依赖菜单是否打开 | 同上；UIA dump 46 节点 |
| 文件/编辑/视图/帮助 | 四个 `Menu` 常驻 | 同上；UIA 39 个 `MenuItem` |
| 菜单项 | 全部非分隔线项：label、keyboard_shortcut、disabled、`Click`+`Focus` | 单测 disabled 与 `chrome::action_enabled` 同源 |
| 菜单项动作 | UIA Click → `App::perform_menu`；禁用项不执行 | 单测 + UIA InvokePattern「设置…」 |
| 设置 / 模态 | `Dialog` + 按钮，`modal=true` | UIA 设置模态 5 节点 |
| 模态隔离 | 打开时菜单栏/正文/面板从树中移除；后台命令拒绝 | 单测 + UIA 实测 46 → 5 → 46 |
| 模态按钮 | UIA Click → `resolve_modal` / `resolve_settings` | 单测 + UIA「取消」关闭 |
| 正文 | `MultilineTextInput`（UIA `Edit`），label「正文」，value = 前 8000 字节有界快照 | UIA dump `ControlType.Edit \| "正文"` |
| 查找 / 大纲 / 快速打开 / 右键菜单 | `Group` / `Menu`；快速打开与命令面板 name 不同 | 单测 |

## 2. 限制（明确的能力边界，不是待办承诺）

1. **正文仅纯文本快照**：只暴露文档前 **8000 字节**，按 UTF-8 字符边界截断；超出部分不可读。
2. **无 caret / selection / 逐行语义**：不暴露光标位置、选区、行结构与按行导航；屏幕阅读器不能按字符/行精读或跟随编辑点。
3. **无文本编辑动作**：未实现 `SetTextSelection`、`ReplaceSelectedText` 等；正文节点只用于导航与朗读快照，不能经 UIA 编辑。
4. **快速打开候选未逐项暴露**：面板只暴露分组名称，候选文件/命令列表未进入语义树。
5. **UIA 节点无屏幕边界**：语义树未提供 bounds，UIA `BoundingRectangle` 多为 `empty`；触摸探索与按坐标命中不可用。
6. **Dialog 的 UIA 类型**：AccessKit `Role::Dialog` 在 Windows UIA 下呈现为 `ControlType.Window`，不是 `ControlType.Pane/Dialog`。
7. **Narrator 真实语音流程 NOT RUN**：无人值守环境无法可靠驱动并核对语音播报，未执行，不冒充通过。
8. **NVDA / 其他 AT NOT RUN**：未在 NVDA、JAWS 或其他屏幕阅读器上验证。
9. **中文 IME 真实候选窗 NOT RUN**：未验证组字与候选窗在无障碍模式下的行为（该限制同时属于既有人工验证遗留项）。
10. **视觉焦点未伪造**：`Focus` 动作只更新语义焦点，不改变可视焦点或菜单 hover 状态。

## 3. 回归基线（本次执行）

| 命令 | 结果 |
|---|---|
| `cargo test -p veil-ui high_contrast` | 1 passed, 0 failed |
| `cargo test -p veil-ui scale` | 14 passed, 0 failed（含 user_zoom 100/120/150/200 语义） |
| `cargo test -p veil-ui command` | 8 passed, 0 failed（键盘命令面板） |
| `cargo test -p veil-ui focus` | 20 passed, 0 failed（焦点可见性/焦点环） |
| `cargo test -p veil-ui anim::` | 4 passed, 0 failed（动画/减少动画路径的快照语义） |

已有人工证据（沿用，不重复宣称）：高对比度真实系统切换 **PASS**、四档 user_zoom **PASS**、系统 DPI 150% 首帧检查 **PASS**，见
[current-head-2026-11-26.md](evidence/ux/current-head-2026-11-26.md) §1–§2。

仍未验证（NOT RUN）：实体物理打印、副屏/多显示器、真实中文 IME 候选窗、Narrator/NVDA 语音。

## 4. 如何关闭

`accessibility` 是 `veil` 的默认 feature。需要不含 AccessKit 的构建时：

```powershell
cargo build --release -p veil --no-default-features --features mermaid-offline
```

注意：`--no-default-features` 会同时关闭默认的离线 Mermaid，因此需显式把 `mermaid-offline` 加回；日常使用不建议关闭。

## 5. 体积

release 产物 **14,362,624 B**，14 MiB 门禁余量 **317,440 B**；AccessKit 实测增量 **+192,512 B**。
详见 [component-inventory.md](../component-inventory.md) §11/§13。
