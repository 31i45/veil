# Veil 架构重构计划：三域分离与文件归位

## Context

阶段二 P1a~P6 已完成性能与局部拆分（块几何索引、视口惰性排版、Frame 单缓冲、shell/ 六文件拆分、配置单键、撤销分组、注解层删除），workspace 444 测试全绿。但代码组织仍违反单一职责：

- `shell/` 3893 行混合三个域：文档生命周期（持久化策略）、派生视图状态（几何/缓存失效）、交互编排（事件路由/绘制）。
- `chrome.rs` 778 行三责合一：菜单模型与命中 / 绘制 / 标签位图缓存。
- `veil-core/src/syntax.rs` 1522 行四责合一：纯数据模型 / 块级扫描 / 行内扫描 / Syntax 状态机。
- 无 `rust-toolchain.toml`，构建不可复现。

**目标**：落地用户给定的目标目录树，使目录结构物理强制「文档真值 → 派生视图状态 → 交互事件」的单向数据流。全程纯搬移、零行为变化，444 测试为安全网，每切片一个可回滚 commit。

## 哲学诊断（为什么这样切）

| 原则 | 应用 |
|---|---|
| 第一性原理 | 视图层 = 三向数据流：文档域（持久化真值）→ 派生域（几何/布局/缓存，纯函数投影）→ 交互域（事件→意图→编辑）。目录即依赖方向的物理约束 |
| UNIX 哲学 | 一文件一职。shell 上帝对象拆解；chrome 按「模型/绘制/缓存」天然接缝三分 |
| 二八定律 | 复杂度 80% 集中在 shell/、layout.rs、chrome.rs；docio/autosave/math/image 已干净，**不动** |
| TDD | 测试随代码内联迁移（colocation），每切片 `cargo test --workspace` 全绿才允许提交下一片 |
| 极简主义 | 删除而非兼容；禁止顺手重构，任何行为变化独立成 commit；净减约 200 行 |

## 探查核实的关键事实（决定可行性）

1. **`BlockIndex`/`BlockGeom` 已存在**于 [geometry.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/shell/geometry.rs) L10-28（top/height/est_h/total_h）——`document/block_index.rs` 是**抽取**而非发明。
2. **统一失效入口已具雏形**：`after_edit()`（[interaction.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/shell/interaction.rs) L11-39）+ `invalidate_reveal()`（geometry.rs L182-223）——`view/cache.rs` 是把散落各处的缓存字段与失效调用**收口**为单一结构。
3. **目标树行数 = 纯代码行**（不含内联 `#[cfg(test)]`）：layout 738 / editor 331 / theme 245 / input 160 / frame 75 / style 92 / table 208 / html 688 经核实**现状代码行已达标**，无需瘦身，只做归位。
4. `shell/tests.rs` 1980 行为集成测试，按被测域拆散**内联回迁**至各目标文件；跨域场景测试留在 `app.rs`。
5. 语法内核四段接缝干净：模型 L16-83、Syntax 状态+reparse L85-274、行内解析 L277-570、块扫描+GFM 表格 L581-958；`lib.rs` 导出面不变，外部零感知。
6. chrome「真实测量宽度」：commit 757ca63 已将文本宽度收敛为单一度量常量——实施 S4 时核实该常量是真实光栅测量还是估算，若是估算则改为经 cosmic-text 测量（作为独立行为 commit，不混入搬移）。

## 目标结构与两处偏差说明

严格按用户目标树执行，仅两处主动偏差：

1. **`bench_typing.rs` 留在 `crates/veil/examples/`**（现状位置）。根 `examples/` 在虚拟 workspace 下不属于任何 package，cargo 不会构建它。
2. **`block_index.rs` 归 `document/`**（按目标树）。理由：BlockIndex 只存 top/height 数值与增量维护，不依赖 cosmic-text/像素概念，属「文档的几何投影」而非排版细节；`view/` 保留依赖字体的排版与光栅。此划分自洽，采纳目标树。

## 执行切片（每片一个 commit，全绿才进下一片）

### S0 · rust-toolchain.toml（独立小事）
- 新建 `rust-toolchain.toml`，以 `rustc --version` 实测值钉死 channel（目标树记为 1.99.1，执行时核实）。
- 验证：`cargo +default build` 工具链解析正确。

### S1 · veil-core：syntax.rs → syntax/ 四分
- `syntax/model.rs`（≈120）：BlockKind/InlineKind/InlineNode/Block/Table/TableCell/CellAlign 纯数据。
- `syntax/block.rs`（≈330）：scan_blocks/scan_one + GFM 表格扫描。
- `syntax/inline.rs`（≈300）：InlineParser 递归下降。
- `syntax/mod.rs`（≈260）：Syntax 状态 + new/inlines/reparse；re-export 模型保持 `lib.rs` 导出面（BlockId/BlockKind/CellAlign/InlineKind/InlineNode/Table/TableCell）零变化。
- 各段内联测试随代码迁移；`tests/core.rs` 不动。
- 验证：`cargo test -p veil-core`（90 单测 + 61 集成全绿）。

### S2 · veil-egui：view/ 成组 + cache 收口（**风险最高片**）
- 平移：layout.rs / frame.rs / style.rs / table.rs → `view/`，修正 use 路径，lib.rs 导出面不变。
- 新建 `view/cache.rs`（≈290）：把 App 中 layouts/rasters/math/table/chrome 标签 5 类缓存字段收为 `Cache` 结构；`after_edit` 的 dirty 失效、`invalidate_reveal` 的光标激活失效、主题切换的全量清缓存（memory 教训：颜色必须入失效集合）统一为 `invalidate_*` 方法族。
- 纪律：失效语义逐条对照现状，Reveal/主题相关测试必须原样通过。
- 验证：`cargo test --workspace` 全绿。

### S3 · veil-egui：document/ 成组
- `shell/document.rs` → `document/doc_state.rs`（≈350）：路径/BOM/脏/基线/自动保存/外部冲突 12 方法平移。
- geometry.rs 的 BlockIndex/BlockGeom + 增量维护方法 → `document/block_index.rs`（≈120）。
- geometry.rs 其余 App 方法（visible_index_range/doc_height/滚动钳制/invalidate_reveal 调用壳）暂留，S5 随 App 归位。
- 验证：`cargo test --workspace` 全绿。

### S4 · veil-egui：chrome.rs → chrome/ 三分
- `chrome/mod.rs`（≈130）：几何布局 + 菜单模型 + 命中矩形（现状 L97-208 区）。
- `chrome/paint.rs`（≈260）：菜单栏/状态栏/滚动条绘制（L337-406 区）。
- `chrome/cache.rs`（≈90）：CachedLabel 标签位图缓存（键=刻度+色+文本，L255-310 区）。
- 检查点：宽度测量真实性（见事实 6），如需改为真实测量，独立 commit。
- 验证：`cargo test --workspace` 全绿。

### S5 · veil-egui：shell/ 消解
- `shell/mod.rs` → `app.rs`（≈400）：App 结构体 + ApplicationHandler + 窗口/主题/度量；geometry.rs 残留 App 方法并入。
- `run()` 装配 → `lib.rs`（→50 行：入口 + 平台能力 trait 声明/注入）。
- `shell/interaction.rs` → 根 `interaction.rs`（≈300）；`shell/paint.rs` → 根 `paint.rs`（≈330）。
- `shell/tests.rs` 1980 行按被测域内联回迁（document→doc_state、cache→view/cache、chrome→chrome/*、跨域→app）。
- 删除 `shell/` 目录。
- 验证：`cargo test --workspace` 全绿（测试总数不得减少）。

### S6 · 收口验证与事实回写
- `cargo test --workspace` 全绿 + `cargo clippy --workspace -- -D warnings` 零警告。
- `cargo build --release`，复测 `veil.exe` 字节数（红线 ≤15 MiB，现基线 5,490,176 B）。
- 回写 `.skills/INSTANCE.md`：最后验证日期、目录结构描述、测试分布（core/egui/veil）、exe 字节数、传递依赖数复测（红线 ≤200，现 121）。
- 冒烟：`pwsh` 启动 veil.exe 打开测试文档，编辑/保存/主题切换人工快速确认。

## 统一纪律

- 每片纯搬移，禁止顺手改逻辑；发现必改的行为问题 → 独立 commit 并说明。
- commit 格式：`refactor(veil-core): syntax 拆分为目录模块` / `refactor(veil-egui): view 域成组与缓存失效收口` 等，沿用现有风格。
- 任一片失败：`git revert` 单片回滚，不带伤前进。
- `veil/` 产品层本轮不动；依赖方向 `veil → veil-egui → veil-core` 不新增反向边。

## 风险

| 风险 | 缓解 |
|---|---|
| S2 缓存收口改得失效时机，引发残留旧帧/旧色 | 失效调用点逐一对照表；Reveal/主题测试钉死；该片单独 commit 可 revert |
| S5 tests.rs 回迁量大（1980 行） | 机械操作，按域分批；总数对账（迁移前后 `cargo test` 计数一致） |
| use 路径大面积修正引入编译噪音 | 每片内一次性修正完再跑测试；lib.rs 导出面保持不变使 veil/ 零改动 |
