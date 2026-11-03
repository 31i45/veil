# ADR-009: 状态模型与架构极致化——单一真值、正交状态机、因果派生

## 状态：已采纳（2026-10-26）

## 背景

2026-10-26 架构审计对 `veil-ui` 状态层给出以下事实（均为当时代码实测，非估算）：

| 事实 | 证据 |
|---|---|
| `App` 为 40 个 `pub(crate)` 顶层字段的无结构聚合，真值 / 持久化布尔 / 派生缓存 / 交互瞬态 / 平台端口 / 窗口资源混在一起 | `crates/veil-ui/src/app.rs` L133–L197 |
| `impl App` 分散在 6 个文件块中，任意字段可被任意域直接读写 | `app.rs:199`、`interaction.rs:18`、`interaction.rs:809`、`paint.rs:11`、`block_index.rs:30`、`doc_state.rs:33` |
| 派生缓存 9 个字段、**7 个手工失效方法**，外部调用点 **30 处**；漏调即脏读，多调即白算 | `crates/veil-ui/src/view/cache.rs` L24–L107 与全 crate 调用点统计 |
| 交互模式优先级隐含在分支书写顺序中（菜单 → 查找栏 → 正文 → 快捷键），新增模式须同时改多处 | `crates/veil-ui/src/interaction.rs` |
| 持久化用 5 个平铺字段（2 个布尔 + 3 个 `Option`）表达 2 个正交维度，交叉出大量非法组合，状态栏优先级靠手写 | `app.rs` 的 `autosave_hold`/`conflict`/`external_saved`/`external_changed`/`external_missing`（相邻的 `baseline` 属真值、`last_activity` 属时间输入） |
| headless 测试路径下 `paint()` 不产生真实呈现，性能探针只覆盖 `apply + after_edit + paint` 的 CPU 部分 | `crates/veil-ui/src/app_tests.rs` 的 `perf_keystroke_path`、`perf_keystroke_10k_blocks` |
| 公共导出 **49 项**，产品层 `veil` 实际只使用 **20 项** | `crates/veil-ui/src/lib.rs` L29–L45；`crates/veil/src` 引用统计 |

这些事实的共同根因不是「代码不好看」，而是缺少一条可判定的状态纪律：

- 没有所有权分类，于是字段只能靠约定约束；
- 没有因果关系，于是失效只能靠人记得调用；
- 没有正交化，于是非法状态可以构造出来；
- 没有纯渲染，于是正确性与性能都无法直接测量。

「基础不牢，地动山摇」在本项目中的具体含义是：**每新增一个功能（查找、大纲、下一个面板），都在扩大上述四项债务，而不是复用已有结构。** 本 ADR 的目的不是重构而重构，而是把状态层收敛到一页可讲完、可由机器校验的最小形态。

## 决策

### 一、状态五分类与唯一派生公式

一切状态必须且只能属于下列五类之一，并在 `App` 结构体注释中登记类别：

| 类别 | 定义 | 允许的操作 | 现有实例 |
|---|---|---|---|
| **真值 Truth** | 唯一来源，重开后仍应存在 | 仅经 `Session::apply` 或显式文档命令修改 | 文本 rope、光标/选区锚点、撤销历史、`path`/`bom`/磁盘基线 |
| **输入 Input** | 外部喂入，应用只读 | 只读；由事件或注入赋值 | 视口尺寸、`scale`、主题模式、系统深色、`now`、IME 组合串、指针 |
| **派生 Derived** | 由真值与输入唯一决定 | 只可重算，不可直接改 | 块索引、布局、光栅、公式图、表格网格、大纲条目、查找命中 |
| **端口 Port** | 能力，不携带状态语义 | 调用 | `Clipboard`/`FileStore`/`MathRenderer`/… |
| **瞬态 Transient** | 一次交互内有效 | 事件内读写，跨帧不得作为真值 | `mods`、`mouse`、`dragging`、`goal_x`、`hover` |

唯一派生公式：

```
frame = f(文档真值, 视口, 缩放, 主题, 交互模式, 指针与焦点)
```

**判定规则：凡既不在该公式右侧、又不在派生缓存键中的视图状态，一律视为缺陷。** 该规则是后续评审与新增字段的准入门槛。

### 二、交互模式显式化为状态机

用类型替代分散的分支顺序，路由成为纯函数、表驱动、可穷举：

```rust
enum Mode { Document, Menu, Search } // 大纲是常驻面板，键盘仍归文档，不进入 Mode
enum TextSink { Doc, Query }         // IME 组合串与提交共用同一落点判定

fn route(mode: Mode, key: Key, mods: Mods, composing: bool) -> Cmd
```

约束：

1. 模式转移只允许发生在 `route`/`perform_action` 两层，事件回调不得再做模式判断；
2. IME `Preedit`/`Commit` 的目标由 `TextSink` 决定，不再散落于多个 `if search.open`；
3. 新增模式 = 新增变体 + 转移表条目 + 测试，不修改既有分支；
4. `route` 只产 `Cmd`（纯函数），执行集中在 `route_key` 一处；测试与真实事件共用同一入口，避免「测的路径不是跑的路径」。

### 三、持久化收敛为单一结构 + 唯一状态投影，时间作为输入

实现审计（2026-10-26 S2 落地时验证）：原「Save × Disk 二维之积」无法行为等价地表达现状——`hold`（自动保存挂起）与 `backup`（已写副本及去重登记）在「自动保存撞上外部修改」与「手工覆盖后」两条路径下不可互相推导；`missing` 又可与「已备份」提示共存。故采用三个独立事实 + 一个时间输入的单结构，非法组合由语义方法收敛：

```rust
struct Persist {
    hold: bool,                // 自动保存挂起（外部冲突未解决）
    pending: Option<Vec<u8>>,  // 待备份的外部版本
    missing: bool,             // 磁盘缺失/不可读
    backup: Backup,            // None | Written(Some(v)) 登记去重 | Written(None) 仅留提示
    activity: Option<Instant>, // 最近内容变更时刻（调度输入）
}
impl Persist { fn status(&self) -> ExternalState } // Changed > BackedUp > Missing，唯一投影
```

约束：

1. 6 个平铺字段（5 个外部状态 + 1 个活动时刻）收敛为 `App.persist` 单字段（`App` 顶层字段 40→35），真值 `baseline` 仍独立；
2. 提示优先级只在 `Persist::status` 实现一次；写入只经语义方法（`begin_external` / `mark_backed_up` / `keep_notice_only` / `clear_backup` / `set_hold` / `reset`），不再直接改布尔；
3. 时间作为输入：`tick(now)` 是外部轮询 → 系统外观 → 自动保存的唯一入口，策略不读墙钟；活动时刻由编辑路径写入、由调度读取；
4. 行为等价由既有 20 项外部变更/自动保存测试 + 6 项 `Persist` 单测（优先级、待备份、登记、重置、活动）共同钉死。
5. 性质级钉死（2026-10-26 补）：外部磁盘事件是全项目唯一不可穷举的输入，故以 proptest 驱动「编辑 / 磁盘写入 / 回退基线 / 删除 / 轮询 / 手工保存 / 自动保存」任意序列（100 组 × ≤30 步），逐步断言六条用户可观察不变量：status 优先级自洽；`hold ⇒ pending 或 backed_up`（挂起必有提示）；自动保存绝不覆盖存在的外部版本；手工保存成功 ⇒ 磁盘==基线且缺失/挂起清除；被覆盖的外部版本必落冲突副本且已写副本永不被改写；连续轮询幂等。另有重放确定性测试（同序列两遍投影完全一致）。断言只观察 `status()`/`hold()`/磁盘副作用，不观察内部事实字段。

### 四、因果键记忆化，取消手工失效

派生缓存从「先写后删」改为「键相等则命中，失配则重算」：

```rust
struct CacheKey {
    block: BlockId,   // 内容因果：脏块重解析即换新 BlockId（syntax/mod.rs:228），天然失配
    reveal: bool,     // 标记展开影响块内排版与光栅
    width_q: u32,     // 版心宽量化
    scale_q: u32,     // DPI 量化
    theme_rev: u32,   // 主题代次
}
struct Memo<V> { key: CacheKey, value: V }
```

落地校准（S3 实测，回写本决策）：

1. **`Revs` 代次承载**：`BlockId` 仅会话内单调、跨会话复用，故主题/会话/图片基准三类外部输入各自单调递增为 `Revs { theme, session, image_base }`。基础键 = `{ width_q, scale_q, theme_rev, session_rev, image_base }`；行内公式按 `{ scale_q, theme_rev }`，图片按 `image_base`。
2. **Reveal 由 bool 改为 64 位激活指纹**：直接以 caret 作键会在块内同点移动时过度重排（引用块同行移动）。改为对引用块行号、列表/分隔线块内激活布尔、每个行内节点的激活位做 FNV 折叠；指纹相等 ⇔ 排版依赖的激活状态相同，命中路径零分配、零 session 调用。
3. **布局键含 `theme_rev` 属缺陷修正**：布局内嵌语法高亮色，旧 `clear_themed` 不清布局会在主题切换后留下陈旧围栏代码色。此项是行为修正而非功能变化。

各缓存的内容因果键：

| 缓存 | 键 | 说明 |
|---|---|---|
| 块布局 / 光栅 / 表格网格 | `BaseKey` + 激活指纹 | `BlockId` 新鲜度已编码内容修订；宽/DPI/主题/会话/图片基准变化即失配 |
| 块级公式图 / 失败记忆 | `BlockId` + DPI + 主题 + 会话 | 失败记忆不含主题（换主题不重试渲染） |
| 行内公式图 | `(tex, scale_q, theme_rev)` | 内容寻址，跨文档天然复用；失败记忆仅按 DPI |
| 图片位图 | `(解析后路径, image_base)` | 另存为/打开改变相对路径基准即失配 |

`Cache` 的 7 个 `invalidate_*`/`clear_*` 方法已全部删除，外部 30 处调用点归零。内存回收由 `Cache::prune` 承担（只删已消失 `BlockId` 与过期代次，不承担正确性——键失配的旧条目即使未回收也不会命中），调用点唯一：索引重建后与块真值同源。迁移期对拍测试 `memo_hit_equals_forced_relayout` 钉死「memo 命中 == 强制重算」的逐块等价。

### 五、渲染纯化与呈现分离

```rust
fn render(&mut self) -> &mut Frame   // 产出完整帧缓冲，只读真值，只填派生缓存
fn present(&mut self)                 // 唯一外部副作用出口：surface 拷贝 + present
```

约束：

1. `render` 对真值只读，只允许填充 `Derived`（memo）；`present` 是唯一产生外部副作用的出口；
2. **幂等性成为正确性属性**：同一输入连续调用两次 `render`，输出帧逐像素相同，以测试断言；
3. 性能探针改为测量真实 `paint = render + present` 路径，修正旧「paint 在 headless 下 no-op」的口径失真；
4. `paint` 的绘制策略不变——本决策只改变入口形态与可测性，不改变像素结果。

落地校准（S4 实测，回写本决策）：

- `render` 不引入 `Inputs`/`Derived` 新抽象层（过度工程化）——直接用 `win_size()` 在 headless 下兜底 800×600 尺寸，surface 存在性只影响 `present` 侧；
- 幂等性成立依赖：滚动钳位、可见区间 chrome 标签、光栅化全部确定性；派生缓存（布局/光栅/标签）第一次 render 填充后第二次全部命中；
- `paint()` 保留为事件循环 Redraw 分支的单入口，内部合成 `render() + present()`；
- 探针口径修正后击键 p95 从手动模拟 4.73ms 上调到真实路径 5.67ms（5005 块），仍在 16ms 预算内。

### 六、端口聚合与公共接口收敛

1. 11 个平台端口字段聚合为单个 `Ports` 结构（能力聚合，不引入框架）；
2. `veil-ui` 公共导出收敛到 ≤ 25 项：仅保留产品层实际使用的端口 trait、`run`、`Doc`、`decode`、端口关联类型（FileEvent / ImageData / MathImage）与 bench_typing 纯排版基准（LayoutEngine / Frame / BlockLayout / BASE_*）；内部类型（Editor / Chrome / Autosave / Viewport / MenuAction / Action / Key / Mods / Style / Table* / translate / html_to_markdown）不导出；
3. 导出面以「产品层需要什么」定义，不以「crate 里有什么」定义。

落地校准（S5 实测，回写本决策）：

- App 顶层字段数从 35（S2 后）降至 26：11 个 `Box<dyn Trait>` 端口并为 1 个 `Ports` 字段；终态目标 12（剩余字段分组为 doc / editor / ui / input / window 等子结构）为后续 S6，不在 S5 范围；
- 公共导出实际数量 25 项，而非 ≤20：bench_typing（产品层纯度量 example）需要 LayoutEngine / Frame 做纯排版与帧分配基准，属"产品层实际需要"，因此纳入；主代码（main.rs / platform/* / export.rs）实际只用 20 项，bench 额外 5 项；
- 产品层 veil 零改动：`run()` 与 `App::new()` 入参形态保持原 10 参数列表（产品层注入端口时无需感知内部 Ports 聚合），内部组装由 veil-ui 层完成；
- 跨 4 个文件（app.rs / interaction.rs / paint.rs / doc_state.rs）的 41 处端口引用统一改为 `self.ports.xxx`；Ports 为 `pub(crate)`，不向公共导出；
- 导出收敛后 veil-ui 内部不再被外部类型引用的内部类型会产生 `dead_code` clippy 警告，属于收敛副产物而非新缺陷（4 条 lib + 3 条 lib test duplicate）。

### 七、明确不做（负空间）

以下方案在本项目中被排除，理由是它们用「更多结构」掩盖「更少不变量」：

- ECS、actor 模型、响应式框架、事件总线、依赖注入容器、插件系统；
- 为拆分而拆文件、无度量支撑的 `unsafe` 优化；
- 引入新依赖来实现状态管理（依赖预算 8/8 与传递依赖 ≤200 不变）；
- 跨切片的批量重构——一切改动按切片独立提交、独立可回滚。

## 落地切片

| 片 | 内容 | 前置 | 验收 | 行为变化 |
|---|---|---|---|---|
| S1 | `Mode` + `route()` 状态机；`TextSink` 接管 IME | 无 | 转移表 100% 覆盖；全量测试绿 | 零 |
| S2 | `Persist` 单结构 + 唯一 `status()` + `tick(now)` | S1 | 6 项 `Persist` 单测 + 既有外部变更测试全绿；`App` 字段 40→35 | 零 |
| S3 | `Memo<Key>` 替换 7 个失效方法 / 30 处调用点 | S1 | memo 与全量重算逐块等价对拍；失效调用点为 0 | 零 |
| S4 | `render`/`present` 拆分 + 探针口径校正 | S3 | 渲染幂等断言；真实上屏路径 p95 有数 | 零（像素不变） |
| S5 | `Ports` 聚合 + 公共 API 收敛 | S4 | 顶层字段 26（35→26，11 端口并为 Ports）；导出 25 项（含 bench_typing 纯排版基准）；产品层零改动 | 零 |

每片要求：`cargo test --workspace` 100% 通过、`cargo clippy` 零警告、单一 commit 可 `git revert`、不改变用户可感知行为。

## 可判定验收（终态）

| 指标 | 现状（S5 后） | 目标 |
|---|---|---|
| `App` 顶层字段数 | 26（35→26，11 端口并为 Ports） | ≤ 12（剩余字段分组为子结构，S6 不在本方案范围） |
| 手工失效调用点 | 0（S3 后，`Memo` 因果键 + `prune`） | 0 |
| 交互模式转移覆盖 | 表驱动 100% 显式（S1 已达成） | 表驱动 100% 显式 |
| 持久化状态 | 单结构 + 唯一 `status()`，写入只经语义方法（S2 已达成） | 同左 |
| 渲染可测性 | headless 下真实跑像素、幂等可断言、真实路径可测 | 同左 |
| `veil-ui` 公共导出 | 25 项（主代码用 20 项 + bench_typing 5 项纯排版基准） | ≤ 25 |
| 新增功能的改动面 | 五类登记 + 一张表 | 同左 |

## 后果

- 好：新增功能只需在五类中登记并给出因果键，不再扩大失效债务；「忘调失效」类缺陷结构性地不再存在；渲染成为可断言纯函数，正确性与性能同时变得可测；公共面由需求决定，后续 API 变更不再误伤内部实现；状态层可用一页文档讲清，符合 UNIX「小即美」与「数据即接口」。
- 坏：迁移有一次性成本（5 个切片）；S3 改动面最大，涉及 `paint` 的 `ensure_*` 编排，需要逐块对拍才能保证等价；S4 需要重写性能探针并重新建立基线数字；迁移期间旧文档（README/INSTANCE）中的相关描述须同步回写，否则产生新的漂移。

## 替代方案

- **维持现状、局部修补**：每次新功能继续增加字段与失效调用点，债务随功能线性增长，与「数十年可维护」目标相反，否决。
- **引入 ECS / 响应式框架 / 事件总线**：把纪律问题转化为框架问题，增加依赖与间接层，且无法解决「因果键」本身，违反依赖预算与「组合而非框架」，否决。
- **全局 generation 全清**：实现最简，但每次编辑、每次主题切换都重算全部派生，直接违反 16 ms 击键预算与惰性排版承诺，否决。
- **大爆炸式重写状态层**：一次提交完成 S1–S5，无法逐片验证等价性，也不能独立回滚，否决。
- **只固化文档不迁移**：文档与代码继续漂移，验收指标无法达成，否决。

## 关联

- ADR-001（rope 与 mark 语义）：真值层的存储基础。
- ADR-003（mark bias 与 affinity）：光标锚点作为真值的规则来源。
- ADR-006（外部文件变更）：本 ADR 决策三将其收敛为 3×4 转移表。
- ADR-007（配置与主题）：`theme_rev` 的吊销来源；主题切换的缓存失效改由因果键承担。
