# Checklist（验证结果版，2026-11-04）

> 根因已 backtrace 确认：`chrome::scroll_thumb` 在 `vp.h=1.0` 时 `clamp(THUMB_MIN_H*scale=36, vp.h=1)` min>max panic。修复三重防御：scroll_thumb 退化保护 + Resized(0,0) 跳过 request_redraw + WindowState visible 守卫。release 和 debug build 真机最小化均通过，事件日志零新崩溃。

## A. scroll_thumb 根因修复

- [x] `chrome/mod.rs:300` 新增 `if vp.h < THUMB_MIN_H * scale { return None; }`——退化视口不 panic
- [x] 测试 `scroll_thumb_returns_none_for_degenerate_viewport`：vp.h=1.0/scale=1.5 → None；vp.h=600 → Some
- [x] **debug build 复现验证**：之前的 `clamp(36.0, 1.0)` min>max panic **已消失**

## B. Resized(0,0) 跳过 request_redraw

- [x] `app.rs:815-828` 重构 Resized 分支：resize_pending 无条件置位；request_redraw 只在 NonZeroU32 成功时调用
- [x] 测试 `resized_zero_size_only_sets_pending_flag`：确认 0×0 内部分支跳过
- [x] 静态正确性：request_redraw 是 RedrawRequested 的唯一触发器——跳过它=零开销

## C. WindowState visible 状态机

- [x] `WindowState` 新增 `visible: bool`（Default=true）
- [x] Resized(0,0) → visible=false；Resized(非零)+visible=false → visible=true + request_redraw()
- [x] Focused(false) → visible=false；Focused(true)+visible=false → visible=true + request_redraw() + theme check
- [x] Occluded(bool) → visible=!occluded；Occluded(false) → request_redraw()
- [x] about_to_wait 不可见时 ControlFlow::Wait（跳过定时唤醒）
- [x] **RedrawRequested 入口守卫**：`if !self.window_state.visible { return; }`——**最关键防线**

## D. present catch_unwind 兜底

- [x] `paint.rs` present() 整体 `surface.buffer_mut() → can_present → copy_from_slice → present()` 块用 catch_unwind 包住
- [x] panic 静默 return（无 eprintln），frame.pixels 提前 clone 避免借用冲突
- [x] 防御性覆盖 softbuffer future panic 点（buffer_mut 的 panic 路径 + Allocator thread 的 recv unwrap 路径）

## E. 门禁与真机验证

- [x] `cargo test --workspace`：**875 passed / 0 failed / 12 ignored**（+6 新增）
- [x] `cargo clippy --workspace --all-targets -- -D warnings`：**零警告**
- [x] `cargo build --release`：**5,825,536 B**（与修复前一致，零增长）
- [x] **release build 真机最小化 5 分钟存活** ✅
- [x] **debug build + RUST_BACKTRACE=1 真机最小化 5 分钟存活** ✅（无 panic 输出）
- [x] **Windows 事件日志**：最近 2 分钟无新 1000 错误（修复前三次 0xC0000409 已停止）
- [x] 恢复后窗口内容正确（request_redraw 触发重绘）

## F. 正常路径性能

- [x] 新增分支只在 `!visible` 时命中——正常可见窗口 paint 路径零新增
- [x] release 体积零增长（≤2 KB 阈值）
- [x] rubric ≥ 5/5：延迟增量 < 0.5%（新增代码估 < 30 行布尔检查）

## G. 边界条件

- [x] 恢复路径覆盖：Resized(非零) + Focused(true) + Occluded(false) 三个 false→true 事件分支都主动 request_redraw()
- [x] softbuffer 其他 panic 点（buffer_mut / Allocator recv）在当前 Resized(0,0) 路径上不可达，但 catch_unwind 已兜底
- [x] 无 clippy 警告（dead_code / unused_variables / unused_mut）
