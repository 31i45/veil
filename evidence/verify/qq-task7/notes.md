# qq-task7 取证说明

环境：Windows 11，主屏 2560×1440，系统 DPI = 100%（`AppliedDPI` 未设置，等效 96 dpi），
`target\release\veil.exe`（2026-11-25 16:17 构建，含本次改动）。截图为 release 版本整窗，
浅色主题（缩放抽样）/ 跟随系统（高对比度）。

## 一、四档等效缩放抽样（zoom 路径）

统一说明：**这是「等效缩放抽样（user_zoom 路径）」，不是系统 DPI 切换。**
Veil 的 `effective scale = dpi_scale × user_zoom`（`crates/veil-ui/src/app.rs` 的 `scale`），
因此改 user_zoom 与改系统 DPI 走同一条几何/排版/光栅路径。快捷键为 `Ctrl+=`。

| 文件 | 实际 user_zoom | 一行说明 |
| --- | --- | --- |
| `scale-100.png` | 1.0 | 等效缩放抽样（zoom 路径，zoom=1.0）；不是系统 DPI 切换。 |
| `scale-125.png` | **1.2** | 等效缩放抽样（zoom 路径，实际 zoom=1.2）；**不是系统 DPI 切换**。产品把 user_zoom 按 0.1 量化（`crates/veil-ui/src/interaction.rs` 的 `zoom_step`），`Ctrl+=` 无法得到 1.25；此图沿用任务槽位名，状态栏可见「缩放 120%」。 |
| `scale-150.png` | 1.5 | 等效缩放抽样（zoom 路径，zoom=1.5）；不是系统 DPI 切换。 |
| `scale-200.png` | 2.0 | 等效缩放抽样（zoom 路径，zoom=2.0）；不是系统 DPI 切换。 |

采集方法：`tools/verify/qq_task7_scale_capture.ps1`（新建脚本），同一进程内依次发送
`Ctrl+=` 0/2/3/5 次（1.0→1.2→1.5→2.0）。100/150/200 用 `PrintWindow(PW_RENDERFULLCONTENT)`；
120 档 `PrintWindow` 返回空白，降级为屏幕捕获，并按 `DWMWA_EXTENDED_FRAME_BOUNDS`
裁掉 DWM 不可见边框，避免混入桌面其它窗口像素。日志见 `scale-capture.log`。

## 二、高对比度真机留证（PASS）

| 文件 | 说明 |
| --- | --- |
| `high-contrast-baseline.png` | 切换前基线（`mode=system`，浅色常态，平均亮度 241.1）。 |
| `high-contrast.png` | 临时开启系统高对比度后 2s 截图：Veil 已切为黑底/黄标题栏/白字（平均亮度 37.4）。 |
| `high-contrast-restored.png` | finally 恢复原值并广播后再截图：回到浅色（平均亮度 241.1）。 |

过程（`tools/verify/qq_task7_high_contrast.ps1`，日志 `high-contrast.log`）：
1. 读取原值：注册表 `Flags=126`，`SPI_GETHIGHCONTRAST` 同步读到 126（高对比度关闭）。
2. 置位：注册表 + `SystemParametersInfo(SPI_SETHIGHCONTRAST)`（127），并广播
   `WM_SETTINGCHANGE("Accessibility")`。
3. **234ms** 后 Veil 完成配色切换（< 1s，符合 1s 轮询预期）；等待总计 ≥2s 后截图。
4. `finally` 恢复 `Flags=126`（SPI + 注册表双写并校验），恢复后 Veil 回落浅色。
5. 事后再次核验注册表 `Flags=126`，系统未停留在高对比度状态。

首次真机尝试（修复前）曾失败：系统已进入 127，但 Veil 4s 内不变色。根因是
`crates/veil/src/platform/theme.rs` 把 `HIGHCONTRASTW.cbSize` 写死为 12；64 位下结构体
实际 16 字节，`SPI_GETHIGHCONTRAST` 直接返回 FALSE（实测 `cbSize=12 → rc=False`，
`cbSize=16 → rc=True, flags=126`），轮询永远读不到高对比度。该次失败已在 finally 中
还原，随后修复并重跑得到上述 PASS。

## 三、像素对齐结论（先测后改）

真实存在的半像素源（1.25×）：`surface::corner_radius` 的 6 × 1.25 = **7.5**，
以及光栅化半径的 2 × 1.25 = **2.5**。1px 分隔线本身以整数像素绘制
（`stroke::HAIRLINE as u32`，未经 scale 相乘），不存在半像素问题。

本次在允许范围内做的改动：
- `geometry.rs`：新增 `snap_to_pixel`（`round()`，幂等），作为统一吸附策略。
- `tokens.rs`：`surface::corner_radius` / `surface::dialog_radius` 乘完 scale 后吸附到
  整数物理像素；1.0× 结果与改动前完全一致（6/8），浅色默认观感不变。
- 回归锁：`tokens.rs::scaled_geometry_snaps_to_integer_physical_pixels`（四档 scale 下
  1px 发丝线、2px 小圆角、浮层/模态圆角均为整数）、
  `platform/theme.rs::highcontrast_struct_size_matches_abi`。
- `platform/theme.rs`：修正高对比度轮询的 `cbSize`（见上）。

未改动的既有半像素点（文件所有权禁止）：`paint.rs` 内联的 `(2.0 * scale)`、
`(2.5 * scale)` 圆角在 1.25× 下仍为 2.5 / 3.125（小圆角抗锯齿略软），
`frame::fill_hover` 的 `corner_radius - 2.0*scale` 仍可能得到 .5 值；这些不在
`geometry.rs`/`tokens.rs` 范围内，未擅自修改。

## 四、门禁

- `cargo fmt --all` 通过。
- `cargo test -p veil-ui` 通过（日志 `test-veil-ui.log`）。
- 附加：`cargo test -p veil highcontrast_struct_size` 通过。
