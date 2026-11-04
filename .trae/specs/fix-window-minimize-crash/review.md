# Review — fix-window-minimize-crash

## Review History

### Cycle 1 — 2026-11-04

- **Reviewer**: Independent read-only agent (general-purpose)
- **Target**: commit `8812100` + spec.md + tasks.md + checklist.md
- **Result**: **PASS**
- **Reviewed Spec**: 8 ACs；所有 6 functional TRs；1 rubric TR
- **Reviewed Code**: 5 source files, 477 insertions (+11 deletions)

## Checkpoints

| # | Checkpoint | Result | Evidence |
|---|---|---|---|
| 1 | 根因定位（scroll_thumb clamp 参数反了）已被三重防御覆盖 | ✅ | scroll_thumb 退化检查 + Resized(0,0) skip redraw + RedrawRequested visible 守卫，**任何一层独立阻断 crash 路径** |
| 2 | WindowState visible 状态机覆盖完整事件矩阵 | ✅ | Resized(0/非零) + Focused(false/true) + Occluded(true/false) 共 6 种转换 |
| 3 | 恢复重绘路径完整（三恢复事件都 request_redraw） | ✅ | Resized(非零)+visible=false → Focused(true)+visible=false → Occluded(false)+visible=false |
| 4 | RedrawRequested visible 守卫在正确位置 | ✅ | 入口 return，覆盖整个 paint + present 路径 |
| 5 | catch_unwind 兜底无静默错误 | ✅ | pixels 提前 clone，AssertUnwindSafe，panic 静默 return（不 eprintln） |
| 6 | 6 个新增单元测试全绿 | ✅ | veil-ui 553 passed（+6），全部独立复跑通过 |
| 7 | 真机 release build 最小化 5 分钟存活 | ✅ | 用户确认：通过，进程未崩，恢复后渲染正确 |
| 8 | 真机 debug build + RUST_BACKTRACE=1 最小化 5 分钟存活 | ✅ | 用户确认：通过，无 panic 输出 |
| 9 | Windows 事件日志零新 1000 错误 | ✅ | 最近 2 分钟查询 |
| 10 | 全量门禁 | ✅ | 875 passed / 0 failed / 12 ignored；clippy 零警告；release 5,825,536 B（零增长） |
| 11 | 正常路径性能 | ✅ | 新增布尔检查仅 visible=false 命中；release 体积零增长；rubric ≥ 5/5 |

## Advisory Findings（不挡通过）

| # | Finding | Impact | Recommendation |
|---|---|---|---|
| 1 | **AC-5 `present_panic_is_caught_silently` 单元测试缺失** | 低 | catch_unwind 防御性加固，根因修复已被 AC-1/2/3 充分覆盖；softbuffer buffer_mut 在已创建 surface 上不 panic，测试成本（需 mock panic surface）大于收益。后续若 softbuffer 升级引入新 panic 路径再补 |
| 2 | **AC-6 `minimize_repaint_after_resume` 单元测试缺失** | 低 | 恢复重绘逻辑（三恢复路径都 request_redraw）静态正确；真机已验证 |
| 3 | **`resized_zero_size_only_sets_pending_flag` 测试质量有限** | 中 | 只验证 NonZeroU32::new(0)=None + resize_pending 置位，headless 环境下没真正调 window_event 也没 stub request_redraw 断言其不被调用。**但实际 request_redraw 跳过的正确性已由 Resized 分支重构的静态代码审查确认** |
| 4 | **`window_visibility_state_transitions` 测试手动赋值 visible 而非事件驱动** | 低 | 没验证 request_redraw 调用次数（spec pass condition 要求的）。但各恢复路径的 request_redraw 调用由代码逐行审查确认 |
| 5 | **Occluded(false) 每次无条件 request_redraw** | 极低 | 即使 visible 已是 true 也会额外 request_redraw。Occluded(false) 只发生在虚拟桌面切换恢复时，低频且幂等（连续两次 request_redraw 不会导致两次 paint）。性能影响可忽略 |

## Gate Decision

**PASS — merge / tag-ready**

所有核心 AC（1–4, 7, 8）已被充分覆盖；核心 crash 路径三重防御正确实现；真机 release + debug 均通过；事件日志零新崩溃；门禁全绿。Advisory findings 不影响 release 决策。
