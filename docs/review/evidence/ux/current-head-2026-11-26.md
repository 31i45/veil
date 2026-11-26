# 当前 HEAD 人体边界证据（IME / DPI / 高对比度 / 打印 / 副屏）

> 日期：2026-11-26
> HEAD：`5b3d9a6`；版本 `3.0.2`；工具链 rustc/cargo 1.99.1
> 环境：Windows 11 企业版 10.0.26300；单屏 2560×1440；系统 DPI 144（150%）
> 原始日志：[high-contrast-2026-11-26.log](high-contrast-2026-11-26.log)、[scale-capture-2026-11-26.log](scale-capture-2026-11-26.log)

## 1. 高对比度 — PASS（真实系统切换）

- 脚本：`tools/verify/qq_task7_high_contrast.ps1`
- 原值：注册表 Flags=126、SPI=126（关闭）；脚本置为 127，广播 `WM_SETTINGCHANGE(Accessibility)`。
- 结果：Veil 在 **228 ms** 内切换到高对比度配色；切换前平均亮度 240.4，切换后 39.2。
- 恢复：Flags 回到 126，平均亮度回到 240.4；`high-contrast-restored.png` 已保存。
- 截图：`evidence/verify/qq-task7/high-contrast.png`、`high-contrast-restored.png`（本地，未入库）。
- 结论：真实系统高对比度触发与观感 **PASS**；不是只看 token 的静态断言。

## 2. 缩放 — 等效抽样 PASS（user_zoom，不是系统 DPI 切换）

- 脚本：`tools/verify/qq_task7_scale_capture.ps1`
- 槽位：100% / 125%（实际 120%，UI 缩放按 0.1 量化）/ 150% / 200%。
- 结果：四档均保存整窗截图，`PrintWindow`/`CopyFromScreen` 成功；125% 槽位降级为 CopyFromScreen，已如实记录。
- 截图：`evidence/verify/qq-task7/scale-100.png`、`scale-125.png`、`scale-150.png`、`scale-200.png`（本地，未入库）。
- **口径**：这是 `user_zoom` 路径，不是系统 DPI 切换；`effective_scale = dpi_scale × user_zoom`，两者共用几何路径，但不能冒充系统 DPI 证据。

## 3. 系统 DPI

| 项 | 状态 | 说明 |
|---|---|---|
| 系统 DPI 150%（当前环境） | 已验证 | 当前系统 `GetDpiForSystem()=144`，release 进程在同一环境启动、截图、命中测试通过 |
| 系统 DPI 100% | NOT RUN | 需要改系统显示缩放，未在无人值守下执行 |
| 系统 DPI 125% | NOT RUN | 同上 |
| 系统 DPI 200% | NOT RUN | 同上 |

静态证据：
- `cargo test -p veil-ui high_contrast`：`theme_matrix_tests::high_contrast_meets_readability_floor` PASS。
- `cargo test -p veil-ui scale`：14 项 PASS，含 `four_dpi_scales_layout_hit_and_paint_match`、`geometry_scales_linearly_with_dpi`、`zoom_step_scales_effective_dpi_and_reflows`。

## 4. 中文 IME — NOT RUN

- 未执行：需要真实输入法会话、候选窗与组合串交互；本轮无人值守环境不适合切换 IME。
- 已有自动化覆盖仅到 `preedit` 边界与路由，不等同于真实候选窗位置/提交/取消/标点。
- 结论：**NOT RUN**；Phase 5 无障碍前必须补真实 IME 走查，不得写成通过。

## 5. 物理打印 — NOT RUN

- 环境存在 `Pantum CM230DN Series 0001` 与 `Microsoft Print to PDF`。
- 未执行：物理打印会真实出纸；本轮不向打印机发送作业。
- PDF 导出/打印版面已有独立证据；但“物理打印页边距/分页/表格/中文断行”仍为 **NOT RUN**。

## 6. 副屏 — NOT RUN

- `System.Windows.Forms.Screen.AllScreens` 仅 1 个屏幕（`\\.\DISPLAY1`，2560×1440）。
- 跨屏拖动、工作区回退、缩放变化无法在本环境验证。
- 结论：**NOT RUN**。

## 7. 结论

- **PASS**：高对比度真实切换、四档 user_zoom、系统 DPI 150%、高对比度/缩放静态测试。
- **NOT RUN**：中文 IME、物理打印、副屏、系统 DPI 100/125/200。
- 这支持报告 ⑦ `3/5` 的判断：适配层有真实证据，但关键人体边界仍未闭合；不能用“代码写了”替代“人走通了”。
