# ADR-005: chrome UI 自绘 + 系统对话框 —— 不引入 egui/GL

## 状态：已采纳（2026-10-25）

## 背景

切片 1 原计划「首个 chrome UI（菜单/状态栏）落地时引入 egui/GL 替换 present 层」。切片 15 真正落地 chrome 时重新评估：现有管线是 winit + softbuffer 软件呈现 + cosmic-text 排版 + 自绘 caret/选区/图片/公式，已有 `Frame` 的矩形填充与位图混合原语；菜单栏、状态栏、滚动条与文件对话框所需的绘制能力是其子集。引入 egui 意味着 glutin/glow 六件套、GL 上下文与事件双轨制，只为画三条矩形。

## 决策

1. **自绘 chrome**：菜单栏/弹出菜单/状态栏/滚动条在 `veil-egui/src/chrome.rs` 纯逻辑实现（几何、命中、滚动换算、状态文本为纯函数），绘制复用 `Frame` 原语，标签复用 `LayoutEngine` 的 preedit 排版 + 光栅化并缓存。
2. **系统文件对话框**：`FileDialog` trait 定义在 `veil-egui`（与 `Clipboard`/`FileStore`/`MathRenderer` 同模式），产品层注入。Windows 实现手写 `GetOpenFileNameW/GetSaveFileNameW/GetActiveWindow` FFI（`crates/veil/src/platform/windows.rs`），**零新增依赖**；非 Windows 返回 None 兜底。
3. **视口化**：内容区 = 窗口减去菜单栏与状态栏；滚动条占右侧 12px；滚轮与滚动条共用 `max_scroll` 钳位。chrome 最后绘制、不透明覆盖，保证内容不侵入 chrome。

## 后果

- 好：直接依赖保持 7/8（winit/softbuffer/cosmic-text/arboard/pulldown-cmark/latex-rust/png；切片 16 文件监听新增 notify 后为 8/8，见 ADR-006），传递依赖不变；无 GL 上下文、无事件双轨；chrome 与原编辑器同一坐标系与 DPI 缩放，缩放/裁剪行为一致；hit-test 与绘制均为 headless 可测。
- 坏：新控件（下拉框、文本框、模态确认框）需要自绘；复杂排版（富文本、动画）不适合。v1 的 chrome 需求只有菜单/状态/滚动条，可接受。
- 约束：字体度量用估算宽（`text_width`）保证命中不依赖上一帧绘制状态；真实标签绘制仍由 cosmic-text 决定，二者在极端字体下可能数像素偏差（v1 接受）。

## 替代方案

- **egui + glutin/glow**：为三条矩形引入六个直接依赖与 GL 上下文，且 softbuffer 呈现层需整体替换；否决。
- **`rfd` crate**：跨平台对话框一行接入，但新增直接依赖并带 GTK/xdg-portal 传递树；与「零新增依赖」目标冲突；否决（手写 FFI 约 100 行即可覆盖 v1 的打开/另存为）。
- **窗口内自绘文件浏览器**：工作量大、体验劣于系统对话框、还要处理路径输入与权限；否决。

## 备注

Win32 `OPENFILENAMEW` 字段顺序按 VISTA+ 定义，`lStructSize = size_of::<OpenFileNameW>()`；过滤器为 UTF-16 双 \0 结尾；保存用 `OFN_OVERWRITEPROMPT`，打开用 `OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST`；owner 取 `GetActiveWindow()`。`FileDialog` 只负责选路径，读写与状态切换仍由 shell 负责（写成功才切换关联路径，失败保留旧路径与脏标）。
