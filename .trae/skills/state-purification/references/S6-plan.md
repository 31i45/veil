# S6 + P1 · 进阶加固计划（ADR-009 后续）

> 状态：**待确认**。S1~S5 已完成，本文件是下一步推进的唯一权威载体。
> 落地位置：`docs/adr/S6-plan.md`（镜像在 skill references 确保上下文压缩后可恢复）。
>
> 修订历史：
> - 2026-10-26 · 初稿，基于 App 26 字段引用次数统计 + 依赖预算审计
> - 2026-10-26 · **S6 完成**：阶段 A dead_code 清零、阶段 B proptest 性质测试（网络不可达时改用纯 std fuzzer 临时方案，恢复后升级为 proptest）、阶段 C C1~C4+C6 子结构归组完成，App 26→10 顶层字段；C5 ViewState（cache+index 归组）因 16 处同时借用结构性冲突跳过
> - 2026-10-26 · **阶段 B+ 完成**：persist 外部变更状态机补两条 proptest 性质（六条可观察不变量 + 重放确定性），测试 525→527 绿

---

## 目标

把 ADR-009 S1~S5 打下的"好结构"变成**不能被推翻的地基**——不是靠注释约定，而是靠：
1. clippy 零警告的经典级底线
2. proptest 性质级测试钉死因果键不变量
3. 类型系统承载状态纪律（App 顶层 ≤12 字段）

---

## 审计事实（2026-10-26 S6 完成后）

| 指标 | 现状 | 来源 |
|---|---|---|
| `App` 顶层字段 | **10**（终态目标达成） | C1~C4+C6 逐步归组后实测 |
| 手工失效调用点 | 0 | S3 后 Memo 因果键 + prune |
| veil-egui 直接依赖 | 8 | Cargo.toml |
| workspace 传递依赖 | 191（红线 ≤200，剩余 9 条） | `cargo tree --prefix none` |
| clippy 警告 | **0** | 阶段 A dead_code 清零 |
| workspace 测试 | **527 绿**（24+98+64+341） | `cargo test --workspace` |
| ADR-009 S1~S5 | 全部落地 | ADR + 代码 + 测试三方一致 |

---

## S6 终态结构（2026-10-26）

```
App {
    doc: DocState       { path, bom, baseline }          // 真值
    window_state: WindowState { window, surface, frame, scroll_y, goal_x, dragging } // 瞬态
    editor: Editor                                      // 暂留顶层
    engine: LayoutEngine                                // 暂留顶层
    index: BlockIndex                                   // 暂留顶层（C5 跳过）
    cache: Cache                                        // 暂留顶层（C5 跳过）
    input_state: InputState { revs, mouse, mods, scale, theme, system_dark, last_theme_check } // 输入
    ports: Ports { clipboard/store/math/images/assets/exporter/dialogs/watcher/config/system/effects } // 端口（已聚合）
    persist_state: PersistState { persist, autosave }   // 持久化策略
    ui_state: UIState { chrome, outline, search }       // UI 组件
}
```

### C5 ViewState 跳过原因

`self.cache` 与 `self.index` 在同一函数内同时被访问的路径 ≥16 处（`refresh_index` 命中判断读 `cache.layout()` 同时写 `self.index.blocks.push()`、`ensure_layouts_range` 遍历 `index.blocks` 同时写 `cache.layouts.insert`、`paint` 两遍收敛遍历 index 同时读 cache 等）。强行合并到同一 `ViewState` 子结构会导致结构性借用冲突——同一次 `&mut self.view` 锁死整个子结构后无法再 `&self.view.cache`（不同字段但同一子结构）。两个字段各自保留顶层，零借用冲突，**零行为变化**。

### App 26 字段引用次数（全 crate 精确统计）

```
editor=213, cache=116, chrome=80, persist=78, scale=64, index=61,
theme=55, search=54, outline=48, engine=36, scroll_y=32, ports=27,
path=26, revs=17, mouse=18, goal_x=11, window=9, frame=9, baseline=9,
bom=6, dragging=6, mods=5, surface=3, autosave=2, system_dark=4, last_theme_check=4
```

---

## 阶段 A · dead_code 清零（P0，30 分钟）

### A1 · 删除 view/mod.rs 多余导出（4 处改动）

| 导出 | 现状 | 修复 |
|---|---|---|
| `PlacedImg` | 内部不用，产品层不用 | 从 pub use 删除 |
| `style_spans, Style` | 内部不用，产品层不用 | 从 pub use 删除 |
| `TableCellLayout, TableLayout, TableRow` | 内部不用（table.rs 内部私有布局），产品层不用 | 从 pub use 删除 |

改后 view/mod.rs 第 17/20/21 行：
```rust
pub use layout::{BlockLayout, InlineAsset, InlineImg, LayoutEngine};
// style/table 模块保持非 pub：内部实现不需要对外暴露
```

### A2 · Autosave::idle 真死代码

- 确认：`cargo grep idle` → 0 调用点
- 修复：删除 `pub fn idle(&self) -> Duration`（autosave.rs 第 30 行）
- 或：如果是预留 API，加 `#[allow(dead_code)]` + 注释说明预留场景

### A 验收

- `cargo clippy -p veil-egui --all-targets` 零 dead_code / unused_imports 警告
- `cargo test --workspace` 全绿（导出减少不影响内部）
- `cargo fmt --all -- --check` 干净

---

## 阶段 B · Proptest 性质测试（P1，1 天）

### B1 · 依赖引入

`crates/veil-egui/Cargo.toml`：
```toml
[dev-dependencies]
proptest = "1.5"    # 仅测试二进制，不进 release，传递依赖 ~30 条（预算 121→~151）
```

### B2 · 性质定义

**性质一：Memo 命中值恒等于强制重算值（任意编辑序列 + 任意主题/DPI 切换）**

```rust
proptest! {
    #[test]
    fn memo_hit_equals_forced_after_any_sequence(
        edit_count in 0usize..50,
        toggles in proptest::collection::vec((0u8..3, 0u8..5), 0..20),
    ) {
        let store = MemStore::default();
        let mut a = app(BASE_DOC, None, false, &store);
        for (i, edit_idx) in edits.iter().enumerate() {
            // 编辑序列：随机在当前 caret 插入/删除单字符
            let dirty = a.editor.apply(...).unwrap_or_else(|| Vec::new());
            a.after_edit(dirty);
        }
        for (theme_act, dpi_act) in toggles {
            // 主题切换 → revs.theme += 1
            // DPI 切换 → self.scale = [1.0, 1.25, 1.5, 1.75, 2.0][dpi_act as usize]
            // base_key() 自然失配
        }
        let before = snapshot(&a);
        a.cache.layouts.clear();    // 命中路径失效
        a.ensure_layouts_all();
        assert_eq!(before, snapshot(&a), "memo 命中值 ≠ 强制重算值");
    }
}
```

**性质二：Render 幂等性（任意序列后连续两次 render 像素一致）**

沿用现有 `render_is_idempotent_consecutive_frames_equal`，扩展到 proptest：
```rust
proptest! {
    #[test]
    fn render_idempotent_after_any_sequence(
        edits in proptest::collection::vec(EditGen, 0..30),
    ) {
        // 编辑序列 → render → 取像素 → render → 再取像素 → assert_eq
    }
}
```

### B 验收

- `cargo test -p veil-egui --lib proptest 2>&1 | Select-String "FAILED"` 空
- 至少 2 条新性质测试通过
- 基线：proptest 种子固定后结果可复现（不用 `std::process::abort`）

### B 反模式防御

| 反模式 | 症状 | 修正 |
|---|---|---|
| 让 proptest 直接操作内部状态（`a.cache.layouts.insert(...)`） | 绕过因果键构造路径 | proptest 只通过 `editor.apply()` / `refresh_theme_visuals()` / `set_scale()` 公共入口驱动 |
| 每次随机种子清一次缓存 | 测试不覆盖"多次编辑后再切换主题"的累积行为 | 同一测试用例内累积 0~50 次编辑 + 0~20 次切换，最后一次性比对 |

---

## 阶段 B+ · 持久化状态机性质测试（2026-10-26 完成）

外部磁盘事件是全项目唯一由不可控进程驱动的输入，因此它是「基础不牢」清单上的最后一项。新增两条 proptest（`app_tests.rs`）：

**事件集**：`Edit`（置脏 + 登记活动） / `DiskChange(v)` / `DiskRevert`（回退基线） / `DiskRemove` / `Poll` / `ManualSave` / `AutoSaveTick`（合成时钟 +2s）。

**性质一 `persist_state_machine_invariants_hold_after_any_sequence`**（100 组 × ≤30 步，逐步断言）：

| 不变量 | 内容 |
|---|---|
| I1 | `status()` 优先级投影自洽：Changed⇒pending、BackedUp⇒!pending∧backed_up、Missing⇒无前两者∧missing、None⇒全清 |
| I2 | `hold ⇒ pending 或 backed_up`——挂起自动保存必有外部状态原因（否则用户看到「不保存」却无提示） |
| I3 | 自动保存绝不覆盖外部版本：磁盘存在且 ≠ 基线时，`AutoSaveTick` 后磁盘字节原样 |
| I4 | 手工保存成功 ⇒ 磁盘 == 基线、缺失态清除、挂起解除 |
| I5 | 外部版本不丢失：被覆盖的外部字节必落冲突副本；已写副本永不被改写 |
| I6 | 连续两次 `Poll` 且磁盘未变 ⇒ 状态事实不变（检测幂等） |

**性质二 `persist_replay_is_deterministic`**：同一事件序列重放两次，五事实 + 磁盘全量内容 + 基线完全一致（排除 HashMap 顺序/时钟漂移/残留状态）。

**断言纪律**：只观察 `status()` / `hold()` / 磁盘副作用，不观察 `pending`/`backup`/`missing` 内部表示——内部表示可变，契约不变。`Edit` 事件直接置脏 + 登记活动而不走 `after_edit`（几何与本次性质无关），使 100×30 步压到秒级。

**已知不做**：「同一外部版本只写一份冲突副本」不可作为不变量——手工保存成功会清空去重登记（`keep_notice_only`），跨轮次同一版本再次出现时无法在不扫描目录的前提下判重；数据安全由 I5 保证，重复副本无害。

---

## 阶段 C · S6 字段分组（P2，2~3 天）

### C1 · 子结构定义（按组总引用升序迁移）

**迁移顺序决策**——先移少引用组，单步 diff 面积最小：

| 步骤 | 子结构 | 字段 | 组总引用 | 改动文件数 |
|---|---|---|---|---|
| C1 | WindowState | surface(3), frame(9), window(9), scroll_y(32), goal_x(11), dragging(6) | 70 | 3~4 |
| C2 | DocState | path(26), bom(6), baseline(9) | 41 | 2~3 |
| C3 | persist 逻辑归并 | persist(78), autosave(2) | 80 | 2~3 |
| C4 | InputState | scale(64), system_dark(4), last_theme_check(4), mouse(18), mods(5), revs(17), theme(55) | 167 | 5~6 |
| C5 | ViewState | index(61), cache(116) | 177 | 3~4 |
| C6 | UI State | chrome(80), outline(48), search(54) | 182 | 3~4 |
| C7 | 剩余顶层 | editor(213), engine(36) | 249 | 5~6 |

**目标终态**：

```rust
pub(crate) struct App {
    doc: DocState,          // Truth：path / bom / baseline
    persist: Persist,       // 持久化事实（已在 S2 收敛）
    autosave: Autosave,     // 调度策略
    view: ViewState,        // 派生：index / cache / revs（代次属于派生键输入）
    input: InputState,      // 输入：scale / mouse / mods / theme / system_dark / last_theme_check
    chrome: Chrome,         // UI 组件（自绘 chrome 状态）
    outline: Outline,       // UI 组件（大纲）
    search: Search,         // UI 组件（查找栏）
    editor: Editor,         // 真值 + 输入混合 → 暂留顶层（Editor 内部复杂）
    engine: LayoutEngine,   // 纯计算组件 → 暂留顶层
    window: Option<Arc<Window>>,   // 瞬态窗口句柄
    surface: Option<Surface<...>>,  // 瞬态呈现缓冲
    ports: Ports,           // 端口（已聚合）
    frame: Option<Frame>,   // 瞬态帧缓冲
}
// 顶层字段数：14（Editor / engine / window / surface 暂不分组）
// 若继续推进到 12：把 persist/autosave 合为 PersistState，把 chrome/outline/search 合为 UIState
```

### C2 · 每步迁移的固定模板

```
# Step C1 WindowState（示例）
1. 在 app.rs 定义子结构：
   pub(crate) struct WindowState {
       pub(crate) surface: Option<Surface<...>>,
       pub(crate) frame: Option<Frame>,
       pub(crate) window: Option<Arc<Window>>,
       pub(crate) scroll_y: f32,
       pub(crate) goal_x: Option<f32>,
       pub(crate) dragging: bool,
   }
2. App 结构体：删 6 个字段 → 加 window: WindowState
3. App::new：按新结构组装默认值
4. 批量替换（按文件）：
   app.rs:  surface/frame/window/scroll_y/goal_x/dragging → window.xxx
   paint.rs: window/surface/frame/scroll_y → window.xxx
   interaction.rs: scroll_y/goal_x/dragging → window.xxx
5. cargo check -p veil-egui --all-targets → 零错误
6. cargo test --workspace → 全绿
7. cargo clippy → 零新警告
8. git commit -m "S6 C1: WindowState 分组（6 字段）"
```

### C 验收

- `App` 顶层字段数从 26 降至 ≤14（Editor / engine / window / surface 暂不分组）
- 类型系统承载五分类纪律（Truth / Input / Derived / UI / Transient 变为真实子结构）
- 行为零变化（全部测试继续绿，clippy 零新增）
- 每步独立 commit，任意一步可 git revert 回滚

### C 反模式防御

| 反模式 | 症状 | 修正 |
|---|---|---|
| 一口气移完所有字段 | diff 过大，出了错定位困难 | 按组总引用升序，每步只移 6~7 字段 |
| 子结构定义跨文件（把 WindowState 定义到 doc_state.rs） | 领域边界模糊 | 所有子结构定义放在 app.rs，与 App 结构体同文件 |
| 替换不彻底（self.window 漏改某些文件） | 编译错误 | cargo check 过了再 commit |
| 子结构加公开访问器（self.window.scroll_y()） | 过度封装，破坏同一 crate 内的直接访问 | 子结构字段继续 pub(crate)，impl App 里直接 self.window.scroll_y 读写 |

---

## 优先级与节奏建议

| 阶段 | 量级 | 前置 | 风险 | 建议时机 |
|---|---|---|---|---|
| A · dead_code 清零 | 30 分钟 | 无 | 极低（纯删除） | 立即（clippy 零警告是经典级底线） |
| B · Proptest 性质测试 | 1 天 | 无（Memo 系统已稳定） | 中（proptest 种子 + 失败分析） | A 之后 |
| C · S6 字段分组 | 2~3 天 | A + B 之后（性质测试先钉住再重构） | 中高（每步引用替换量大） | B 之后 |

**推荐节奏**：A 今天下午即可落地；B 需要安静环境（proptest 调试是脑力密集），安排一个完整工作日；C 可以分片推进（每天 2~3 步，每步独立 commit）。

---

## 验证矩阵（每个阶段结束后必须全部通过）

```
cargo fmt --all -- --check
cargo check -p veil-egui --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets
cargo tree -p veil-egui --prefix none | wc -l
```

---

## 回滚策略

- 每阶段（A / B / C）独立一个 commit；C 内部每步一个 commit
- 任何阶段失败 → `git revert <step-commit>`，回到上一阶段末态
- 性质测试（B）可以先加但不跑（`#[ignore]`），避免调试阻塞主开发
