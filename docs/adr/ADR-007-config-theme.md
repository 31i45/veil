# ADR-007: 配置与主题 —— 手写 TOML 子集 + 系统外观/窗口材质接缝

## 状态：已采纳（2026-10-25）

## 背景

切片 17 需要落地 INSTANCE §5 输入点 ⑦（配置文件）与 §7 接缝项（系统主题跟随、窗口材质）。直接依赖已达预算上限 8/8（ADR-006 后果），`serde`+`toml` 不可用；主题颜色此前硬编码在 `chrome.rs`/`shell.rs`，深色外观下公式仍为黑色、无法阅读。需要决定：配置格式与解析方式、主题模型、系统外观与 Mica 的实现边界。

## 决策

1. **配置解析：手写极简 TOML 子集，零新增依赖**。唯一支持的配置项：
   ```toml
   [theme]
   mode = "system"   # system | light | dark
   ```
   支持注释（引号内 `#` 不算）、空行、`[section]`、单双引号字符串；未知键/段/裸值计入 `ignored`，启动后在状态栏提示「配置有 N 处无法识别，已忽略」。重复键后者生效。严格小写模式值。文件非法 UTF-8 不整体拒绝（按 lossy 解码后逐行解析，坏值自然进入 ignored）。
2. **主题模型：`Theme` 调色板是唯一颜色来源**。`ThemeMode::{System,Light,Dark}` + `Theme::of(mode, system_dark)` 解析出最终色板；浅色值与切片 15/16 的硬编码常量**逐字节一致**（视觉不回退），并在单测中钉死。正文/代码块/选区/表格/菜单/状态栏/滚动条/弹出菜单全部改读 `Theme`。
3. **系统外观：注册表轮询，仅「跟随系统」时启用**。`SystemTheme` trait 由产品层注入，Windows 读 `HKCU\...\Themes\Personalize\AppsUseLightTheme`（`RegGetValueW`，advapi32）。主线程在 `about_to_wait` 每 1s 校验一次（仅当 mode == System 且窗口存在），窗口 `Focused(true)` 时立即重查以消除滞后。读不到（非 Windows/键缺失）按浅色处理。
4. **窗口材质：DWM 深色标题栏 + Mica，失败静默**。`WindowEffects` 接收 `NativeWindow { hwnd }`（Win32 HWND 由 winit `window_handle()` 提供，trait 不暴露 winit 类型）。实现设置 `DWMWA_USE_IMMERSIVE_DARK_MODE`（20，旧版回退 19）与 `DWMWA_SYSTEMBACKDROP_TYPE = MICA`（38）。**内容区始终由 `Frame` 绘制不透明底色**，因此 Win10/无 Mica 时自动退化为纯色外观，无需调用方分支；Mica 只作用于 DWM 绘制的标题栏/边框区域（softbuffer 不提供 alpha 合成，内容区透明 Mica 需 DirectComposition，列为非目标）。
5. **配置落盘：`ConfigStore` trait，产品层原子写**。路径 `%APPDATA%\Veil\config.toml`（其余平台 `$XDG_CONFIG_HOME/veil` 或 `$HOME/.config/veil`）；`save` 先 `create_dir_all` 再同目录 tmp + rename。文件不存在属正常状态（首次运行）→ 静默默认；保存失败保留本次主题并提示「配置未保存」。
6. **主题切换即整体刷新颜色派生缓存**。文本光栅、表格位图、块级/行内公式图、chrome 标签缓存全部作废重建（布局几何与图片位图不受影响）；`MathRenderer::render` 增加 `color` 参数（latex-rust `PngOptions.color`），深色主题公式与正文同色。菜单栏新增「视图」菜单（跟随系统/浅色/深色），当前项右侧显示「当前」。
7. **接缝装配：`ThemeEnv` 一次性注入**（config/system/effects 三项），避免 `run` 参数继续膨胀；测试用 `ThemeEnv` 内存实现保持 headless 可测。

## 后果

- 好：零新增直接依赖，预算仍为 8/8；主题颜色单一来源，浅色视觉不回退；深色下正文/公式/表格/菜单可读且通过 WCAG 对比度断言；配置损坏只提示不失效；系统切换在 1s 内跟手（窗口聚焦即时）；保存原子，不产生半写配置。
- 坏：不支持的 TOML 语法（数组/表内表/转义字符串）会被当作无法识别并忽略；系统外观轮询在「跟随系统」时约 1 次/秒唤醒；Mica 仅覆盖 DWM 边框区域，内容区非透明。
- 约束：Mica 内容区 / 亚克力背景 / 自定义标题栏（客户区自绘）均需合成层或额外 Win32 子系统，v1 非目标；主题仅含模式，不含自定义色值（后续如需自定义色板，先提升配置格式并补 ADR）。

## 替代方案

- **`serde` + `toml`**：更通用，但直接依赖 8→10，违反 ADR-006 预算约束；否决。
- **Windows 注册表以外的系统主题 API（`ShouldSystemUseDarkMode` 等）**：依赖 `uxtheme` 与 Win10 版本差异，注册表读取已足够且更稳定；否决。
- **监听 `WM_SETTINGCHANGE`**：winit 未暴露该事件，需要子类化窗口过程（`SetWindowSubclass`），复杂度高于 1s 轮询；否决。
- **内容区透明 + 分层窗口实现真实 Mica**：需 `WS_EX_LAYERED`/DirectComposition 与自管 alpha 合成，softbuffer 不兼容；放弃，列为非目标。
- **主题切换时不失效光栅缓存并给公式图着色叠加**：缓存键不含颜色、公式为位图无法可靠重着色；整体失效重建更简单且正确。
