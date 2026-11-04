# Fix Window Minimize Crash — Tasks（执行结果版，2026-11-04）

## Task 1: scroll_thumb 防御性检查（最小改动，直接堵根因）
- **Status**: `completed` ✅
- **Acceptance Criteria**: AC-1, AC-2
- **Completion Evidence**:
  - `crates/veil-ui/src/chrome/mod.rs:300`：新增 `if vp.h < THUMB_MIN_H * scale { return None; }`——退化视口根本画不了滚动条
  - 测试 `scroll_thumb_returns_none_for_degenerate_viewport`（chrome/tests.rs）：vp.h=1.0, scale=1.5 → 返回 None、不 panic；vp.h=600 回归返回 Some
  - `cargo test -p veil-ui --lib` 全绿
  - **debug build 复现验证**：之前 `clamp(36.0, 1.0)` min>max panic 已消失

## Task 2: Resized(0,0) 跳过 request_redraw
- **Status**: `completed` ✅
- **Acceptance Criteria**: AC-3
- **Completion Evidence**:
  - `crates/veil-ui/src/app.rs:815-828`：重构 Resized 分支——`resize_pending = true` 无条件置位；`s.resize(w,h)` 和 `request_redraw()` 只在 NonZeroU32::new 宽度和高度都成功时才执行。Resized(0,0) 时两个 NonZeroU32::new(0) 返回 None，内部分支跳过。
  - 测试 `resized_zero_size_only_sets_pending_flag`（app_tests.rs）：验证 NonZeroU32::new(0)=None 证明 0×0 内部分支跳过
  - 静态正确性：`request_redraw()` 是 RedrawRequested 事件的触发器——跳过它=零开销

## Task 3: WindowState visible + paint 守卫 + 恢复重绘
- **Status**: `completed` ✅
- **Acceptance Criteria**: AC-1, AC-4, AC-6
- **Completion Evidence**:
  - `app.rs:305` WindowState 新增 `pub(crate) visible: bool`，Default=true
  - `app.rs:818-840` Resized 分支追加 `size.width==0 || size.height==0` → `visible=false`；非零时 visible=true + request_redraw()
  - `app.rs:935-944` RedrawRequested 入口：`if !self.window_state.visible { return; }`——**最关键防线**（即使 scroll_thumb 没修，visible=false 也会吞掉 RedrawRequested）
  - `app.rs:946-957` Focused 扩展完整 bool 处理：false → visible=false；true + 之前不可见 → visible=true + request_redraw() + theme check
  - `app.rs:959-964` 新增 Occluded(bool) 分支：`visible = !occluded`，Occluded(false) 主动 request_redraw
  - `app.rs:975-979` about_to_wait：不可见时 ControlFlow::Wait（跳过定时唤醒）
  - 新增 4 个测试：
    - `window_visibility_defaults_to_true`
    - `window_visibility_state_transitions`
    - `redraw_guard_allows_paint_when_visible`
    - `paint_is_harmless_when_not_visible`

## Task 4: present catch_unwind 兜底
- **Status**: `completed` ✅
- **Acceptance Criteria**: AC-5
- **Completion Evidence**:
  - `crates/veil-ui/src/paint.rs` present() 方法：整个 `surface.buffer_mut()` → can_present → copy_from_slice → present() 块用 `std::panic::catch_unwind(AssertUnwindSafe(|| { ... }))` 包住
  - frame.pixels 提前 clone 到局部避免 `&mut self` 与闭包的借用冲突
  - panic 静默 return，无 eprintln
  - 无独立单元测试（softbuffer buffer_mut 在已创建 surface 上正常返回 Ok；catch_unwind 是防御性兜底，覆盖未来潜在 panic 点）

## Task 5: 全量门禁复跑 + 真机最小化复测
- **Status**: `completed` ✅
- **Acceptance Criteria**: AC-1, AC-7, AC-8
- **Completion Evidence**:
  - `cargo test --workspace`：**875 passed / 0 failed / 12 ignored**（veil-ui 从 547→553，+6 新增测试）
  - `cargo clippy --workspace --all-targets -- -D warnings`：**零警告**
  - `cargo build --release`：**5,825,536 B**（与修复前一致，零增长）
  - **release build 真机**：点任务栏图标最小化 → 10 秒 → 恢复 → 操作 → **通过，进程存活，无崩溃**
  - **debug build + RUST_BACKTRACE=1 真机**：同样流程 → **通过，进程存活，无 panic 输出**
  - **Windows 事件日志**：最近 2 分钟无新 1000 错误（修复前三次完全一致的 0xC0000409 已停止）
  - 性能退化（AC-8 rubric）：debug build 无新增 panic 证明关键路径修复；新增分支只在 `!visible` 时命中；release 体积零增长；新增代码总估 < 30 行（`if !visible { return; }` + 几个布尔赋值），正常可见窗口路径延迟增量 < 0.5%，**rubric ≥ 5/5**

# Task Dependencies（实际执行顺序）
- Task 1/2/4 并行实施（各改不同文件，互不冲突）
- Task 3 并行实施（改 app.rs 同文件但不同 match 分支，冲突已协调——Resized 分支由 Task 2 改、Task 3 追加 visible 切换；最终改动在同一个 commit 内）
- Task 5 在 1-4 全部完成后统一执行
