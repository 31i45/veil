# Fix Window Minimize Crash — Product Requirements Document

## Overview

- **Summary**: 修复 Veil 在 Windows 上点击任务栏图标尝试最小化时的崩溃问题。根因已精确定位：`chrome::scroll_thumb` 在 `vp.h=1.0`（最小化时 `win_size()` 返回钳位到 1×1）时调用 `clamp(THUMB_MIN_H*scale=36.0, vp.h=1.0)` 导致 min>max panic。
- **Purpose**: v1 发布前消除已复现的崩溃路径；同时建立窗口生命周期事件处理的防御性模式。
- **Target Users**: 所有 Windows 11 用户。点击任务栏图标、按 Win+Down 都会触发。

## Goals

- 窗口最小化后**绝不崩溃**，恢复后正确重绘。
- `chrome::scroll_thumb` 等几何计算函数对退化视口尺寸（vp.h < 最小滑块）有防御性处理。
- 修复不影响正常可见窗口的渲染路径与性能。

## Non-Goals

- 不引入 v2 功能（窗口过渡动画、最小化到托盘）。
- 不升级 softbuffer / winit。
- 不改变 WindowEvent 分派主结构。
- 不修复 release build 下 `0xC0000409` 栈破坏以外的其他潜在崩溃（若存在独立于本次触发链的其他 softbuffer 风险，列为后续修复）。

## Background & Context

### 根因（已 backtrace 确认）

**debug build**：
```
thread 'main' panicked at f32.rs:1605:9: min > max, or either was NaN. min = 36.0, max = 1.0
backtrace #3: veil_ui::chrome::scroll_thumb  at chrome/mod.rs:299
```

**release build**（Windows 事件日志，三次完全一致）：
```
异常代码：0xC0000409 (STATUS_STACK_BUFFER_OVERRUN)
错误偏移：0x00000000002b45f5（veil.exe 内部，同 crash point）
```

**调用链**：
1. 用户点任务栏图标最小化 → Windows 发送 `Resized(0,0)`
2. `app.rs:816-820`：`NonZeroU32::new(0)` 返回 None，**跳过** `surface.resize()`（好！）
3. 但仍置 `resize_pending = true` 并 `request_redraw()`（漏洞）
4. Windows 发出 `RedrawRequested` → `app.rs:925` → `paint()` → `render()`
5. `app.rs:662` `win_size()`：宽高为 0 时钳位到 `NonZeroU32::new(1).unwrap()` → (1,1)
6. `paint.rs:694` render 调 Chrome::paint → scroll_thumb
7. `chrome/mod.rs:299`：`let h = (vp.h * vp.h / doc_h).clamp(THUMB_MIN_H * scale, vp.h);`
8. `THUMB_MIN_H * scale = 24 * 1.5 = 36.0`，`vp.h = 1.0` → **clamp(36.0, 1.0)** → min>max **panic!**

**release 模式下的差异**：`f32::clamp` 在非 debug 构建中不做 min>max 检查，直接返回**错误值**（第一个参数被当作 max，第二个被当作 min——数学上 clamp(36, 1) 返回 36）。这个错误值 36.0 被用于后续栈上操作，触发 `__security_check_cookie` 检测到的栈破坏 `0xC0000409`。

### 为什么不是 softbuffer

softbuffer 的 `buffer_mut` panic 点已确认存在（win32.rs:274），但在当前 Resized(0,0) 路径上**不可达**：buffer 在上一次非零 resize 时已创建，`resize_pending` 被 apply_pending_resize 合并时又跳过了 0×0——所以 softbuffer buffer_mut 不会触发 panic。`present()` 里的 `can_present(frame.pixels.len(), target.len())` 也挡住了 1×1 帧 vs 旧 buffer 的不匹配。

### 现有缺失（已确认）

1. **无窗口可见性状态**：`WindowState` 无 `visible` 字段；`Resized(0,0)` 后仍跑完整 render。
2. **scroll_thumb 无退化保护**：`if doc_h <= vp.h + 0.5 { return None }` 检查通过（doc_h > 1.0 几乎恒成立），但 `vp.h < THUMB_MIN_H * scale` 时直接 panic。
3. **winit 0.30 无 Minimized 事件**：`window_event` match 只有 Focused(true)（做 theme check），Focused(false) 落入 `_ => {}`；Occluded(bool) 完全落入 `_ => {}`。

### 红线影响

- 产物体积 ≤15 MiB（当前 5.56 MiB）：预计增 0–几 KB。
- clippy 零警告。
- 性能：正常可见窗口路径零分支增。

## Functional Requirements

- **FR-1**: `scroll_thumb` 必须在 `vp.h < THUMB_MIN_H * scale` 时提前返回 None（或安全值），**不得 panic**。
- **FR-2**: 应用必须在 Resized(0,0) 时**跳过 request_redraw()**（最小化窗口不需要重绘请求）；只置 `resize_pending = true`（恢复时能正确合并 resize）。
- **FR-3**: 应用必须监听 Focused(false) 与 Occluded(bool) 事件，维护 `WindowState::visible: bool` 状态。
- **FR-4**: 当窗口处于不可见状态（visible=false）时，`paint()` 路径必须提前返回。
- **FR-5**: 窗口从不可见恢复（Focused(true) / Occluded(false) / Resized(非零)）时，必须主动 `request_redraw()` 触发重绘。
- **FR-6**: `present()` 对 softbuffer panic 加 `catch_unwind` 兜底（防御性加固，本次崩溃根因不在此但必须修）。

## Non-Functional Requirements

- **NFR-1**: 正常可见窗口的 paint 路径延迟**不超过 1%** 变化。
- **NFR-2**: 新增分支不得引入 clippy 警告。
- **NFR-3**: release 产物体积增长 **≤2 KB**。

## Constraints

- **winit 0.30 无 Minimized 事件**：最小化检测靠 Resized(0,0) 侧推。
- **`win_size()` 在 0×0 时钳位到 1×1**：不可改（影响渲染主路径），需在上游处理。

## Assumptions

- Focused(false) 或 Occluded(true) 在最小化时会发送（作为 visible=false 的补充信号）。
- Resized(非零) + visible 之前为 false → visible=true + request_redraw() 能覆盖恢复路径。

## Acceptance Criteria

### AC-1: 崩溃消失（根因修复）
- **Type**: `rule`
- **Given**: 应用打开后处于可见状态
- **When**: 用户点击任务栏图标使窗口最小化
- **Then**: 进程持续存活；恢复后窗口内容正确；debug build + RUST_BACKTRACE=1 下 5 分钟无 panic/unwinder；release build 5 分钟无 `0xC0000409`
- **Pass Condition**: 多次最小化/恢复循环后进程存活
- **Evidence**: 真机操作 + 事件日志无新 1000 错误

### AC-2: scroll_thumb 防御性检查
- **Type**: `rule`
- **Given**: `vp.h = 1.0`（退化视口）
- **When**: 调用 `scroll_thumb(vp, doc_h, scroll, scale)`
- **Then**: 返回 None（或安全值），**不 panic**
- **Pass Condition**: 单元测试构造 vp.h=1.0 断言返回 None
- **Evidence**: `scroll_thumb_returns_none_for_degenerate_viewport` 测试

### AC-3: Resized(0,0) 不触发 RedrawRequested
- **Type**: `rule`
- **Given**: Resized(0,0) 到达
- **When**: window_event 处理完成
- **Then**: resize_pending 置 true，但**不**调用 request_redraw()
- **Pass Condition**: 单元测试断言 Resized(0,0) 不产生额外 RedrawRequested
- **Evidence**: `resized_zero_size_skips_redraw_request` 测试

### AC-4: 窗口可见性状态机
- **Type**: `rule`
- **Given**: WindowState 有 `visible: bool` 字段
- **When**: 依次 Resized(0,0) → Resized(非零) → Focused(false) → Focused(true) → Occluded(true) → Occluded(false)
- **Then**: visible 正确切换为 false → true → false → true → false → true；每次 false→true 时 request_redraw() 被调用
- **Pass Condition**: 单元测试断言 visible 变化与 request_redraw 调用次数
- **Evidence**: `window_visibility_state_machine` 测试

### AC-5: softbuffer panic 兜底
- **Type**: `rule`
- **Given**: 构造一个让 present() 里 panic 的场景
- **When**: 该 panic 发生
- **Then**: catch_unwind 捕获，panic 被静默吞掉，进程继续
- **Pass Condition**: 单元测试断言不 panic 不 abort
- **Evidence**: `present_panic_is_caught_silently` 测试

### AC-6: 恢复后正确重绘
- **Type**: `rule`
- **Given**: 窗口最小化期间累计 dirty 状态
- **When**: 用户恢复窗口
- **Then**: 第一帧就是正确内容
- **Pass Condition**: 恢复后 render 输出正确
- **Evidence**: `minimize_repaint_after_resume` 测试

### AC-7: 门禁不回归
- **Type**: `rule`
- **Given**: 修复完成后
- **When**: `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo build --release`
- **Then**: 测试 ≥ 869 passed / 0 failed；clippy 零警告；release 体积增量 ≤ 2 KB
- **Pass Condition**: 三条命令全部通过
- **Evidence**: 命令原始输出摘要

### AC-8: 正常路径零性能退化
- **Type**: `rubric`
- **Dimension**: 正常可见窗口 paint 路径延迟变化
- **Scale**: 1–5
- **Anchors**: 1 = 增量 > 5%；3 = 增量 1–5%；5 = 增量 < 0.5%
- **Pass Threshold**: ≥ 4
- **Evidence**: perf_keystroke_attribution 10.5k 块档 frame_compose p95 对比
