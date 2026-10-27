---
name: state-purification
description: Two-stage architecture purification for stateful render systems. Use when replacing manual cache invalidation with causal-key memoization, or splitting render from present or IO side effects. Do not use for trivial single-file refactors without state or cache.
---

# State Purification

两步纯化模板：先把派生缓存从"手工失效驱动"改成"因果键记忆化"（S3），再把渲染入口从"计算 + 副作用混合"拆成"纯计算 render + 副作用 present"（S4）。适用场景：编辑器/文档查看器/UI 框架——任何有块结构、派生缓存、像素呈现的系统。

## 触发信号（满足其一即启动）

1. 代码里存在 `invalidate_*` / `clear_*` / `drop_*` 等手工失效方法族，且调用点散落全 crate（≥5 处）
2. render 方法里既读真值、又写派生缓存、又拷贝 surface / 发事件 / 刷新窗口
3. headless 下 render 是 no-op，性能探针手动模拟 ensure 路径（口径失真）
4. 主题切换 / DPI 变化 / 打开新文档时缓存"局部清"，留下潜在陈旧（如围栏代码块高亮色不改）

## 阶段一：因果键记忆化（S3）

### 骨架

```rust
// 键 = 外部输入代次 × 量化内部状态
#[derive(Clone, Copy, PartialEq, Eq)]
struct BaseKey {
    width_q: u32,      // round(width × 100)
    scale_q: u32,      // round(scale × 1000)
    theme_rev: u32,
    session_rev: u32,
    image_base: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct LayoutKey { base: BaseKey, reveal: u64 }  // 激活指纹
#[derive(Clone, Copy, PartialEq, Eq)]
struct MathKey   { block: BlockId, scale_q: u32, theme_rev: u32, session_rev: u32 }

struct Memo<K: PartialEq, V> { key: K, value: V }
impl<K: PartialEq, V> Deref for Memo<K, V> { type Target = V; fn deref(&self) -> &V { &self.value } }

struct Revs { theme: u32, session: u32, image_base: u32 }  // 全部原子递增
```

### 三步落地

1. **删除全部 invalidate / clear 方法族**，调用点归零。派生缓存统一改为 `HashMap<Key, Memo<K, V>>` 查询，命中走 `.matches()` 或 `cache.layout(id, &kind, range, base, caret).is_some()` 判布尔
2. **在唯一重建入口加 prune**（本项目是 `refresh_index`）：只删已消失 `BlockId` 与过期代次条目。prune 不承担正确性——键失配的旧条目即使未回收也不会命中
3. **App 顶层加 `revs: Revs`**：主题切换 → `revs.theme += 1`；新文档 → `revs.session += 1`；另存为 → `revs.image_base += 1`；DPI 变化 → 由 `scale_q` 量化键自然失配

### 必须避开的坑

| 反模式 | 症状 | 修正 |
|---|---|---|
| `BlockId` 跨会话复用却无 `session_rev` | 开新文档后偶发命中旧布局 | 块级键全部带 `session_rev`，行内公式按 tex 寻址故不需要 |
| 直接用 caret 作 Reveal 键 | 块内同点移动时整块重排、光标视觉跳 | 改 64 位 FNV 折叠激活指纹：引用块行号、列表/分隔线激活布尔、每个行内节点激活位 |
| prune 承担正确性 | 想靠 prune 主动删"可能陈旧"的条目，逻辑与键判断分离 | prune 只删已消失 block / 过期代次；正确性全由键 == 输入因果保证 |
| 主题切换不影响布局 | 围栏代码块高亮色在主题切换后陈旧 | 布局键必须含 `theme_rev`；布局内嵌语法色，主题是布局的因果输入 |
| 失败记忆键含主题 | 换主题时重渲块级公式（昂贵）且 stderr 重复 | 失败记忆（`MathFailKey`）键刻意不含 `theme_rev`，保持"换主题不重试"语义 |

### 测试契约

- `memo_hit_equals_forced_relayout`：清缓存前后逐块字段（height / reveal / imgs / layout_text）必须字节相等
- `prune_keeps_only_current_generation`：代次变化四段语义（主题 / DPI / 新会话 / 图片基准）逐段钉死
- `system_mode_follows_live_switch_and_clears_color_caches` 从直接 insert 改为 `Memo{ key: raster_key, value }` 形式

## 阶段二：render / present 纯化（S4）

### 骨架

```rust
impl App {
    pub(crate) fn render(&mut self) -> &mut Frame {
        let (w, h) = self.win_size();          // headless 兜底 800×600
        // ensure_* 两遍收敛 → scroll clamp → 全部绘制
        self.index.blocks.is_empty().then(|| self.refresh_index());
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        // 两遍收敛：检查可见块 layout 键是否全部命中
        let range = self.visible_index_range();
        if self.index.blocks.get(range).is_some_and(|bs| bs.iter().any(|g| self.cache.layout(g.id, &g.kind, g.range, base, caret).is_none())) {
            self.ensure_layouts_visible();
            self.recompute_geometry();
        }
        // ...frame.reset + 全部绘制...
        self.frame = Some(frame);
        self.frame.as_mut().unwrap()
    }

    pub(crate) fn present(&mut self) {
        let Some(frame) = self.frame.as_ref() else { return; };
        let Some(surface) = &mut self.surface else { return; };  // headless → no-op
        if let Ok(mut t) = surface.buffer_mut() {
            t.copy_from_slice(&frame.pixels);
            let _ = t.present();
        }
    }

    pub(crate) fn paint(&mut self) { let _ = self.render(); self.present(); }  // 事件循环单入口保留
}
```

### 三步落地

1. **拆分入口**：旧 `paint()` 一分为二——render 只产出 Frame、present 只拷 surface；`paint()` 保留为事件循环 Redraw 分支的合成入口
2. **headless 真跑像素**：render 里取尺寸用 `win_size()`（有 surface 取真实尺寸、无 surface 兜底 800×600），surface 存在性只在 present 侧判
3. **性能探针口径修正**：把手动模拟（ensure + recompute）改成直接调 `paint()`，让真实 render→present 路径进入计时范围

### 必须避开的坑

| 反模式 | 症状 | 修正 |
|---|---|---|
| render 里造 `Inputs` / `Derived` 新抽象层 | 过度工程化，App 已持有全部状态 | 直接用 App 字段；memo 查询是 render 里最昂贵的操作，零抽象开销 |
| 幂等性假设不成立 | 第二次 render 像素变了（chrome 标签第一次才填满） | 幂等性依赖：派生缓存（布局/光栅/标签）第一次 render 填充后第二次全部命中；两遍收敛确定性；scroll clamp 确定性 |
| render 中直接 `surface.buffer_mut()` | headless 不跑、present 与 render 耦合 | surface 操作全挪 present；render 只碰 `self.frame` |

### 测试契约

- `render_is_idempotent_consecutive_frames_equal`：同一输入连续两次 render，`frame.pixels` 逐元素相等（Frame.pixels 是 `Vec<u32>`，每像素一个 u32）
- `present_is_noop_without_surface_and_isolated_from_render`：无 surface（headless）时 present 后 frame 状态不变
- 性能探针：`perf_keystroke_path` 与 `perf_keystroke_10k_blocks` 全部调真实 `paint()`，p95 ≤ 16ms 预算

## 交付清单

完成纯化后必须全部满足：

- [ ] 7 类失效方法（`invalidate_dirty / invalidate_reveal / clear_themed / clear_resize / clear_scale / clear_repath / clear_document`）与全部调用点归零
- [ ] workspace 全量测试 100% 通过、`cargo clippy --workspace` 零警告、`cargo fmt --all` 干净
- [ ] 新增测试：`memo_hit_equals_forced_relayout`、`render_is_idempotent_consecutive_frames_equal`、`present_is_noop_without_surface_and_isolated_from_render`
- [ ] ADR 文档回写**实测校准**（不是写一份静态设计）：主题键是否含 theme_rev、Reveal 是 bool 还是指纹、prune 定位等必须标记为"实测回写"
- [ ] INSTANCE.md 红线表格更新真实性能数字（口径修正后击键 p95 通常比手动模拟高 ~15%，属正常成本）

## 本项目实例参考

- 代码：`crates/veil-egui/src/view/cache.rs`（Memo / Revs / BaseKey / prune）、`crates/veil-egui/src/paint.rs`（render / present / paint）、`crates/veil-egui/src/app.rs`（`revs` 字段）
- ADR：`docs/adr/ADR-009-state-model.md`（决策四 + 决策五 + 落地校准小节）
- 关键数字：S3 后 521 测试 → S4 后 523 测试；击键 p95 4.73ms（手动模拟）→ 5.67ms（真实 render→present 路径），均在 16ms 预算内
