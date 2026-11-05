---
name: "veil-walkthrough-verify"
description: "Veil 真机走查与发布门禁：基线测试/clippy/体积/bench、env 探针插桩、Windows GUI 自动化与截图取证、台账回填与反证流程。当用户要求真机走查、复现排版/交互缺陷、跑发布门禁或回填走查台账时调用。"
---

# Veil 真机走查与发布门禁

把「验证」当作独立阶段：先取证、再归因、后修改；未观察到真机证据前，任何缺陷不得标记「通过」。

## 1. 基线门禁（每次改动后必跑）

在仓库根目录 `d:\VSCode\Veil` 执行：

```powershell
cargo test --workspace 2>&1 | Select-String "test result:"
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release
(Get-Item target\release\veil.exe).Length
cargo run --release -p veil --example bench_typing
```

验收口径（以当前基线为准，变化时同步文档）：
- 测试：911 passed / 12 ignored，0 failed（共 923 项用例，`cargo test --workspace`）；新增用例数如实登记。
- clippy：零警告。
- release exe ≤15 MiB（基线 5,856,256 B）。
- `keystroke_core` p95 <16 ms（2026-11-05 实测 7.12 ms）。
- 冷启动（进程→稳定窗口）与内存用 `walkthrough.ps1 -Measure` 采集。

## 2. 探针插桩规则

- 只用环境变量门控，默认路径零开销（`OnceLock` 只读一次 env）。
- 探针只做观测，不改变排版/编辑结果。
- 测毕必须回退探针改动（`git restore <file>`），除非探针本身是交付物并已评审。
- 探针输出走 stderr，便于 `2> file` 收集；不要引入日志框架。

排版类探针示例：`VEIL_BREAK_PROBE=1` 输出视觉行首/尾禁则违规。
注意：cosmic `LayoutRun.text` 是整条逻辑行，视觉行首/尾必须取 `glyphs.first()/last()` 的区间；
只有同一逻辑行存在前/后视觉行时，首/尾才由折行决定。

## 3. Windows 真机自动化

启动与取证：

```powershell
$err = Join-Path $env:TEMP 'veil-probe.txt'
$env:VEIL_BREAK_PROBE='1'
$p = Start-Process target\release\veil.exe -ArgumentList "`"$file`"" -RedirectStandardError $err -PassThru
```

窗口控制与滚动（Win32 P/Invoke）：

```powershell
Add-Type @"
using System; using System.Runtime.InteropServices;
public class VW {
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr a,int x,int y,int cx,int cy,uint f);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int c);
}
"@
# 等 MainWindowHandle 非零；SetWindowPos(..., 320, 900, 0x0004)
# PageDown: PostMessage(hwnd,0x100/0x101,VK_NEXT=0x22,0)
```

- 宽度扫描优先在同一进程内连续 `SetWindowPos`（每次等待 ≥350ms），避免反复启动。
- 覆盖窄窗口（240–800px）与深滚动（≥20 次 PageDown）；只做单页截图不足以覆盖长文。
- 截图：先用 `PrintWindow`（`PW_RENDERFULLCONTENT=2`）；自绘窗口常返回全黑，必须做空白检测，再临时置顶 + `CopyFromScreen`（不调用 `SetForegroundWindow`）。
- 滚动：无焦点窗口收不到 `WM_KEYDOWN`，会用截图哈希发现「画面未变」，此时回退 `WM_MOUSEWHEEL`（`wParam` 高字为 delta）；仍未变化必须记 FAIL，不能当深滚动证据。
- 现成脚本：`tools/walkthrough.ps1 -Doc <md> -Widths 240,320,480,800 -ScrollPages 20 -Probe -Measure`，产物在 `%TEMP%\veil-walkthrough\evidence\run-*`。
- **鼠标交互**：后台注入无效（2026-11-05 实测：`PostMessage`/`SendMessage` 的 `WM_LBUTTONDOWN` 不触发 winit 命中测试；无焦点窗口的 `WM_KEYDOWN` 也无效）。**前台注入有效**：`-Steps "click:x,y;key:{TAB};shot:name"` 用 `SetForegroundWindow` + `SetCursorPos` + `mouse_event` + `SendKeys`，执行前后保存/恢复前台窗口与光标位置（会短暂抢焦点，需用户授权）。点击坐标是 client 坐标，但本应用窗口 chrome 约 70px，line 1 对应 client y≈60（先截一张图标定再写坐标）。滚轮 `WM_MOUSEWHEEL` 是唯一有效的后台注入。拖放与系统对话框仍需人工。
- 断言优先用探针文本；截图用于确认视觉表现，不做像素级 golden（DPI/字体差异会误报）。
- 自动化不得抢占用户工作窗口；测试结束必须 `Stop-Process -Id $p.Id -Force`。

## 4. 归因顺序（反证清单）

1. **编辑/序列化层**：先写探针验证 `session.serialize()` 与源文本一致，排除编辑路径。
2. **排版/几何层**：用 headless 测试固定版心、真实字形度量；再真机宽度扫描。
3. **渲染/标记显隐层**（reveal）：检查是否源码标记未隐藏、缓存键是否折叠 caret 依赖。
4. **绘制/呈现层**：检查主题色、光栅缓存失效、DPI。
5. 归类后再改代码；每类修复必须配回归测试，并重跑第 1 节门禁。

常见误判：
- 单测通过≠真机正确（单测版心与真机宽度/字体不同）。
- 硬行首/硬行尾的标点属用户原文，不是禁则违规。
- 表格单元格不使用 cosmic 断行：`shape_cell` 先按字形簇测宽 + 禁则边界选断点生成硬行，再 `Wrap::None` 排版；探针以 `include_hard=true` 检查所有视觉行边界。
- 探针误报先修探针，不要据此改产品逻辑。

## 5. 台账回填规则

- 结果写入 `docs/review/3e-walkthrough-results.md`，只登记可复现证据（命令/宽度/文档/输出）。
- 未复现保持「未解决」，并写清已排除的路径与下一步触发条件。
- 修复项附测试名与 commit；不回填未实测数字。
- 规格与任务同步 `.trae/specs/<change-id>/tasks.md`、`checklist.md`。

## 6. 禁止事项

- 不加防御层掩盖问题；先找根因。
- 不 fork 依赖、不引第三方排版引擎来绕过局部缺陷。
- 不在未观察前把「未解决」改成「通过」。
- 不提交临时探针/临时测试文档（除非作为交付物）。
