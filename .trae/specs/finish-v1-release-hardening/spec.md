# Veil v1 发布前加固（崩溃清零 + 击键路径归因）Spec

## Why

v1 发布前的状态是"红线达标但余量薄、且已知缺陷未清零"：10.5k 块击键端到端 p95 已记录为 **12.9ms**（预算 16ms，仅剩 19%），5k 块 6.65ms；同时代码复核确认两个**可直接触发 panic** 的路径（文末输入 `$$`、窗口最小化后 present 长度失配），以及一组边界不一致（GFM 删除线内链接、撤销分组跨光标移动合并）。需要一份以**实测数据决策**、可回滚、逐条可验证的收尾计划，而不是再一轮"理想形态"重构。

## 现状评估：`脏块重排版 + 增量解析` 是否最佳实现

已核对的代码事实：

| 环节 | 现状 | 评估 |
|---|---|---|
| 增量解析（`Syntax::reparse`） | 脏区块级重扫、向前扩展、列表区域回走、delta 对齐、zip-reuse 保 id | ✅ 机制正确，符合"只改 1 块"的语义 |
| reparse 收尾 | `build_refs`（O(#blocks) 全量遍历）+ `rebuild_id_index`（O(#blocks) HashMap 重建） | ❌ 与编辑范围无关的全量工作，随块数线性增长 |
| 视图索引 | `refresh_index` 每击键全量重建 `BlockIndex`；`Session::blocks()` 对每块做 2 次 `offset_of_line` | ❌ 同上；这是 p95 里最可疑的固定成本 |
| 脏块重排版 | `layout_block` 每块新建 cosmic-text `Buffer` + 整块 `set_rich_text` + `Shaping::Advanced` | ✅ 已是库能力下限（cosmic-text 不支持行级增量 shape），单块受 64KiB/512 行硬上限 |
| 光栅 | 按 `BlockId` 缓存，仅脏块重光栅；`rasterize` 为 O(块像素) | ✅ 合理 |
| 公式/表格/图片 | 独立缓存与可见区惰性排版 | ✅ 合理 |

**结论**：不是最佳实现，但**优化空间集中在"索引与辅助结构的全量重建"，不在 shaping**。shaping 属必要成本，除非自研排版引擎（与"极简"直接冲突，不做）。因此本 spec 的第一步不是改代码，而是**给击键路径装上分项计时**，用数据决定是否值得动 `Block` 形态。

## What Changes

- **P0 崩溃清零**（3 项，均为已确证或高置信度的运行时缺陷）
- **击键路径分项探针**（release + `#[ignore]`，5 段归因）
- **数据驱动的索引增量化**（条件任务：探针占比超阈值才实施，含"不改"选项）
- **行内解析边界修正**（P2：`~~` stop 边界内的括号配对）
- **撤销分组显式断开**（`Session::break_undo_group`）
- **Transaction 收敛**（`Vec<PrimEdit>` → `PrimEdit`，不单独立项、并入上文改动）
- **架构裁决留痕**：wgpu 否决与触发条件（ADR-011）、批次 3 降级为按需清单、批次 2-5c 关闭
- **3e 走查收尾计划**（剩余 50 项 + 未修缺陷 R2/T1/I1 的执行与回写）

**BREAKING**：无对外 API 破坏性变更。`Session` 新增一个方法（加法）；`Transaction` 为 `pub(crate)`，形态变化不影响外部；`run(doc, ports)` 已在此前收敛。

## Impact

- Affected specs：性能红线（INSTANCE §2 p95 ≤16ms）、3e 走查、批次 3、ADR 体系
- Affected code：
  - `crates/veil-ui/src/paint.rs`（present 长度守卫、探针接线）
  - `crates/veil-ui/src/app.rs`（Resized 处理、索引重建入口）
  - `crates/veil-core/src/syntax/mod.rs`（`math_source` 守卫、`reparse` 收尾、`build_refs` 门控）
  - `crates/veil-core/src/syntax/inline.rs`（`scan_bracket_pairs` 上界）
  - `crates/veil-core/src/session.rs`（`break_undo_group`、条件性 Events 扩展）
  - `crates/veil-core/src/history.rs`（Transaction 收敛）
  - `crates/veil/examples/bench_typing.rs`（分项探针）
  - `docs/adr/`、`docs/review/3e-walkthrough-results.md`、`docs/review/3e-manual-walkthrough.md`、`.skills/INSTANCE.md`、`README.md`

## ADDED Requirements

### Requirement: 崩溃免疫（P0）

系统 SHALL 在下列三个输入/事件下不 panic、不异常退出，且行为符合降级语义。

#### Scenario: 文末输入 `$$`
- **WHEN** 文档以 `$$` 结尾且无尾随换行（用户逐字输入到文末），该行被识别为 Math 块
- **THEN** `math_source` 返回空串或 `None`（按"内容为空"处理），不 panic

#### Scenario: 单行 `$$x$$` 与 `$$$$` 回归
- **WHEN** 文档含 `$$x$$`
- **THEN** 得到内容 `x`；`$$$$` 仍按段落处理，与既有测试一致

#### Scenario: 窗口最小化后 present
- **WHEN** `Resized(0,0)` 后帧尺寸（钳为 1×1）与 surface 缓冲长度不等
- **THEN** `present` 丢弃该帧（no-op），不 `copy_from_slice` panic；恢复窗口后首帧正常

#### Scenario: resize 事件风暴
- **WHEN** 连续拖拽改变窗口尺寸产生大量 `Resized`
- **THEN** 不阻塞主线程接受后续键盘事件；索引重建在单次红绘前合并为一次（不改变正确性）

### Requirement: 击键路径可归因

系统 SHALL 提供 release 下可分项的击键计时探针，输出 p50/p95 的 5 段耗时：`core_apply+reparse` / `index_rebuild`（`blocks()`+`refresh_index`）/ `dirty_layout` / `rasterize` / `frame_compose`。

#### Scenario: 探针产出可比较基线
- **WHEN** 在 5k / 10.5k / 20k 块文档上运行探针
- **THEN** 输出各段 p50/p95（µs/ms）与块数，并写入 `docs/review/` 下实测记录（供门槛判定与回滚比对）

### Requirement: 索引增量化（条件实施）

系统 SHALL 依据探针数据决定是否实施索引增量化，且实施后的红线不劣化。

#### Scenario: 判定阈值
- **WHEN** `index_rebuild` 段占击键 p95 ≥ 20%，或 20k 块文档 p95 > 16ms
- **THEN** 实施 `Block` 字节区间 + 后缀 delta 平移 + `Events` 增量字段（"批次 3 #1+#2"合并立项）
- **WHEN** 上述阈值均未命中
- **THEN** 不实施，仅保留探针与实测记录（明确"不改"的裁决）

#### Scenario: 增量化正确性
- **WHEN** 实施后
- **THEN** 既有 852 项测试全绿、clippy 零警告、CommonMark 652 + GFM 23 无回归；10.5k 块 p95 ≤ 10ms、20k 块 ≤ 16ms

### Requirement: GFM 删除线边界一致

系统 SHALL 使行内括号配对只在当前 `parse_until` 的 stop 边界内生效。

#### Scenario: 跨 stop 的括号不成链
- **WHEN** 输入 `~~a [b c~~ d](url)~~`
- **THEN** 删除线在第一个 `~~` 闭合（内容为 `a [b c`，其中 `[` 为纯文本），其后 ` d](url)~~` 为纯文本，不产生跨越 stop 的 Link

#### Scenario: 删除线内正常链接不受影响
- **WHEN** 输入 `~~[x](u)~~`
- **THEN** 删除线内含 Link，行为与现状一致

#### Scenario: 代码段优先于删除线
- **WHEN** 输入 `` ~~a `b~~` c~~ ``
- **THEN** 代码段内的 `~~` 不作为删除线闭标记（`backtick_runs` 不受 stop 上界裁剪）

### Requirement: 撤销分组可显式断开

系统 SHALL 提供显式断组入口，使视图层在光标移动/选区变化后开启新撤销步。

#### Scenario: 光标移动后不合并
- **WHEN** 输入 `ab` → 按方向键（或移动光标）→ 输入 `c` → 撤销
- **THEN** 仅撤销 `c`，`ab` 保留

#### Scenario: 连续输入仍合并
- **WHEN** 无光标移动，连续输入 `abc`
- **THEN** 一次撤销全部回退（既有连续性合并语义不变）

### Requirement: 架构裁决留痕

系统 SHALL 把"不做"的裁决固化为可查证文档，避免重复讨论。

#### Scenario: wgpu 裁决
- **WHEN** 查阅 ADR-011
- **THEN** 明示：v1 不引入 wgpu；不纳入默认路线图；给出触发条件（4K/120Hz 滚动实测掉帧且 CPU 呈现为主因、内容区 Mica/亚克力、文档级过渡动画、GPU 侧公式/图片实时缩放）；前置动作 = `spikes/` 独立 spike + ADR 立项

#### Scenario: 批次 3 重新定级
- **WHEN** 查阅批次 3 清单
- **THEN** 由"里程碑"降级为"按需触发清单"：#1/#2 与性能探针阈值绑定；#3 已做；#4 已并入；#5 已完成；#6 仅改文档口径

#### Scenario: 批次 2-5c 关闭
- **WHEN** 查阅该条
- **THEN** 标注"不做"并给出理由：`build_refs` 每击键仅 O(#blocks) 次 kind 比较，收益在噪声级；增量化需处理引用定义 first-wins 全局序，风险在语义级

## MODIFIED Requirements

### Requirement: `Events` 契约（条件修改）

原描述：`dirty_blocks: Vec<BlockId>` 未表达删除块/合并拆分。

新描述：正确性已由"BlockId 新鲜分配 + 视图按 alive 集合 prune"覆盖，**不作为正确性缺陷处理**；仅在索引增量化立项时，`Events` 追加索引增量所需信息（块表代次或字节 delta + 受影响区间），用于视图局部更新。

### Requirement: 3e 走查口径

原描述：走查剩余 50 项暂不推进（假定走查已完成）。

新描述：v1 发布前必须完成剩余 50 项（C/D/E/F/G/H 六区）的人工验证并回写结果；未修的 R2/T1/I1 纳入本 spec 执行；纯人工项（打印、系统主题、Mica、IME、盲测）标注 `needs-user`。

## REMOVED Requirements

### Requirement: 批次 2-5c `build_refs` 增量

**Reason**：收益在噪声级（10k 块约 10k 次 enum 比较/击键），增量化需维护"引用定义 first-wins 全局序"，属正确性风险而非零成本重构。
**Migration**：无。若未来探针显示该段占比 > 5%，再以探针数据重新立项。

### Requirement: 批次 3 作为独立里程碑

**Reason**：六项中 0 项改善任何红线；#3 属 UX 细节、#4 属整洁度、#1/#2/#6 为投机性优化，不应以"理想形态"驱动改动。
**Migration**：拆为"按需触发清单"并入本 spec；#3/#4 本次落地，#1/#2 与探针阈值绑定，#6 仅改文档口径。

### Requirement: wgpu/GPU 渲染（v1 及默认路线图）

**Reason**：体积（≤15MiB，现 5.56MiB）与依赖（≤200，现 141）两条硬约束会被 wgpu 家族直接挤压；且它不解决真正的瓶颈（shaping/解析仍是 CPU）；p95 12.9ms 的余量应投在索引层。
**Migration**：v1 保持 `softbuffer` + CPU 光栅；触发条件满足时按 ADR-011 立项。
