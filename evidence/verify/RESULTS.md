# AC9 / AC11 验证结果（已定稿）

> 2026-11-25 最终状态：AC9 = 3/3 通过；AC11 取消，AccessKit 无障碍功能按产品决定整块移除。
> 本文件中 AC11 部分保留为决策审计材料，不再作为产品验收依据。

## 0. 环境与构建

| 项 | 值 |
|---|---|
| 系统 | Windows 11 企业版，Build 26300，x86_64 |
| 硬件 | Intel i9-14900HX / 32 GB / Samsung NVMe 1 TB |
| DPI | 150% |
| 工具链 | rustc / cargo 1.99.1 |
| 最终默认构建 | `cargo build --release --bin veil` → 14,109,184 B（13.46 MiB），SHA-256 `21E35417360729B0F07CDC815E1AEBBFA67ACEFB9F8E511EAFC19CDF180383E4` |
| 合成输入 | `System.Windows.Forms.SendKeys` + `SetForegroundWindow`，`send_keys` / `click_menu` 记录在证据 header |

## 1. AC9 结论：3/3 通过

用户 2026-11-25 盲评三项全为 1：

| 项 | 结果 | 备注 |
|---|---|---|
| I-1 滚动条胶囊 thumb | 1 | 深色主题下旧构建的滚动条宽度异常在最新构建中已正常，以最新构建为准 |
| I-2 菜单快捷键列与无分隔线 | 1 | 快捷键右对齐成一条竖线，无 1px 分隔线 |
| I-3 模态 vs 浮层三项差异 | 1 | 底色 + 阴影 + 圆角同时不同 |
| **合计** | **3/3** | 阈值 2，通过 |

评审披露：由开发者本人评分，属**非盲评**；参照物（`charmap.exe`，记事本在本机仅 WindowsApps 别名）只用于判断“是否与系统默认相同”，不参与得分。

## 2. AC9 证据清单（`evidence/verify/ac9/`）

均为 PrintWindow 截图（`PW_RENDERFULLCONTENT`），无降级；主题由平均亮度交叉验证。

| 文件 | 尺寸 | 平均亮度 | 判定主题 |
|---|---:|---:|---|
| `20261125-light-full.png` | 1372×1106 | 239.1 | light |
| `20261125-light-chrome.png` | 1372×102 | 239.1 | light |
| `20261125-light-scrollbar.png` | 30×1050 | 239.1 | light |
| `20261125-light-menu.png` | 1372×1106 | 241.6 | light |
| `20261125-light-modal.png` | 1372×1106 | 212.2 | light |
| `20261125-dark-full.png` | 1372×1106 | 40.1 | dark |
| `20261125-dark-chrome.png` | 1372×102 | 40.1 | dark |
| `20261125-dark-scrollbar.png` | 30×1050 | 40.1 | dark |
| `20261125-dark-menu.png` | 1372×1106 | 40.5 | dark |
| `20261125-dark-modal.png` | 1372×1106 | 35.8 | dark |
| `*-menu-masked.png` / `*-modal-masked.png` | 1372×1061 | — | 裁掉顶部 45 px 非客户区 |
| `reference/20261125-light-charmap.png` | 735×638 | — | 参照物 |

菜单弹层由 `-ClickMenu` 合成点击展开；模态由 `-SendKeys "^v%{F4}"`（粘贴一个字符置脏后 Alt+F4）打开。

## 3. AC11 结论：取消，功能已移除

用户在真实 Narrator 下观察到的结果：只朗读应用标题与文档标题，随后提示“……区域，关闭扫描”，无法进入菜单与正文。结论：对真实用户无价值。

在此之前的自动取证（仅作审计留档，`evidence/verify/ac11/`）：

- 修复了启动 panic、动作未接线、菜单不可达、模态不隔离四个缺陷（commit `f1271a6`、`4362045`）。
- UIA Invoke 端到端自测 T1–T4 全部 PASS：菜单项 Invoke 后整窗亮度 249.9 → 42.1；禁用项 Invoke 抛异常且无导出；模态按钮 Invoke 后树从 5 节点恢复 45 节点；脏文档 Invoke「新建」弹出确认后取消复位。
- 已知限制：所有节点 `BoundingRectangle` 为空（触摸/鼠标浏览不可用）、设置模态无按钮、正文无语义文本节点。

产品决定（2026-11-25，用户批准）：不再修补，整块删除。commit `bd3b03e`（10 文件，+21 / −1682）：

- 删除 `crates/veil-ui/src/accessibility.rs`、`tools/verify/ac11_uia_dump.ps1`；
- 移除 `accessibility` feature、`accesskit`、`accesskit_winit`、`Cargo.lock` 中的 22 个传递依赖、`THIRD-PARTY-LICENSES.md` 中 4 行；
- `shell.rs` 回到 `impl ApplicationHandler<Doc>`（删除 `ShellEvent`、adapter 字段、语义树推送、5 个 accessibility 测试）；`app.rs` 恢复单一 `create_window`；
- `docs/manual-test-veil.md` 删除 §J，§I 改用默认构建，并保留一行事实说明“产品不提供屏幕阅读器支持”。

回归：`cargo test -p veil-ui` 1331 passed、`cargo test -p veil` 234 passed；clippy exit 0；`cargo fmt --check` 通过；默认 exe 14,109,184 B 未变；3 秒 smoke 存活、退出码 0；`--features accessibility` 报 feature 不存在。

## 4. 评分总表

| 项 | 得分 | 阈值 | 结论 |
|---|---|---|---|
| AC9 视觉签名辨识度 | 3/3 | ≥2 | **通过** |
| AC11 Narrator 流程 | — | ≥2 | **取消**（功能移除，用户批准） |

## 5. 复现命令（AC9）

```powershell
cargo build --release --bin veil
$exe = 'target\release\veil.exe'

pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region menu -ClickMenu -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region menu -ClickMenu -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region modal -SendKeys "^v%{F4}" -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region modal -SendKeys "^v%{F4}" -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region scrollbar -Doc .\README.zh-CN.md -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region scrollbar -Doc .\README.zh-CN.md -OutDir evidence\verify\ac9
```
