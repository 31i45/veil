# Tasks

> 本文件为**提案任务清单**，全部未勾选，等待用户决策后再执行。
> 阶段 1/2 含硬门禁，只有满足触发条件才可开工；未满足时不得擅自推进。

## 阶段 0：断行策略纯函数化 + 禁则单一真值（建议批准）

- [x] Task 0.1: 建立断行策略纯函数模块 `crates/veil-ui/src/view/linebreak.rs`
  - [x] SubTask 0.1.1: 定义边界判定契约：行首/行尾禁则集合 + `glue_before/after` + `glue_spans`；不依赖 cosmic-text、无 IO、无全局状态（实施修订：需要 advance 的“行走装配”改由阶段 1 承担，阶段 0 先固化判定面）
  - [x] SubTask 0.1.2: 实现胶水段生成（文首/硬行首不注入、连续胶水字符逐个粘连、行尾不注入右粘），保持确定性
  - [x] SubTask 0.1.3: 在 `view/mod.rs` 注册模块；公开面仅 `pub(crate)`，不影响 `lib.rs` 导出面

- [x] Task 0.2: 禁则表提升为唯一真值
  - [x] SubTask 0.2.1: 将 `LINE_START_FORBIDDEN` / `LINE_END_FORBIDDEN` 移出 `#[cfg(test)]`，作为实现与测试共用的常量
  - [x] SubTask 0.2.2: 删除实现侧 `is_glue_left` / `is_glue_right` 的“7 字符子集”语义，改为由禁则表派生胶水决策
  - [x] SubTask 0.2.3: 校验与 UAX#14 的关系：对已由 CL/OP/EX/NS 天然覆盖的字符不重复注入，避免无谓 WJ 缝隙

- [x] Task 0.3: 注入逻辑改由策略产出
  - [x] SubTask 0.3.1: `build_typo_subs` 的胶水段生成改为消费策略输出；`Sub`/`OffsetMap` 结构保持不变
  - [x] SubTask 0.3.2: 中西文间距逻辑保持不变，二者区间仍互不重叠
  - [x] SubTask 0.3.3: 保留 WJ 作为阶段 0/1 的过渡机制，不做结构性移除

- [x] Task 0.4: 回归与黄金语料
  - [x] SubTask 0.4.1: 新增纯函数单测：行首禁则、行尾禁则、省略号/破折号粘连、UAX 覆盖字符不重复注入
  - [x] SubTask 0.4.2: 新增 CJK 禁则回归（混排、行首/行尾、恰好满行、±4px 版心），期望值按字形实算
  - [x] SubTask 0.4.3: 断言胶水段确定性，并覆盖文首/硬行首/行尾边界
  - [x] SubTask 0.4.4: 全量 `cargo test --workspace` 保持绿（新增后 900 passed / 12 ignored，新增 6 条）

- [ ] Task 0.5: C2 归因探针与真机复测（需用户配合）
  - [x] SubTask 0.5.1: 增加显式探针（环境变量 `VEIL_BREAK_PROBE` 门控，默认关闭）：输出每视觉行首/尾字符与违规文本
  - [x] SubTask 0.5.2: 真机归因已执行：02 文档 240–800px 连续宽度 + 深滚动、01 长文 300px 深滚动，正文段落 0 违规；表格单元格窄列宽下复现 `》` 行首违规（探针实证）
  - [x] SubTask 0.5.3: 归因结论已回填 `docs/review/3e-walkthrough-results.md` C2 行：正文段落路径当前不复现；C2 至少存在于表格单元格路径（`shape_cell` 用 `Wrap::Glyph`）；状态保持「未解决」，不填「通过」
  - [ ] SubTask 0.5.4: 正文段落的原始复现场景需用户补充（窗口宽度/文档/截图）；确认后才能判定该路径是否仍需修复

- [x] Task 0.6: 阶段 0 门禁验证
  - [x] SubTask 0.6.1: `cargo clippy --workspace --all-targets -- -D warnings` 零警告
  - [x] SubTask 0.6.2: release 构建成功，exe 体积 5,844,480 B（+3,584 B，仍 ≤15 MiB）
  - [x] SubTask 0.6.3: 击键延迟 bench 无回退：`keystroke_core` p95 1.07 ms <16 ms（bench_typing，5005 块）
  - [x] SubTask 0.6.4: 确认 `crates/veil-core/**` 零改动，`veil-ui` 外部依赖数仍为 4（未新增任何依赖）

- [x] Task 0.7: 修复表格单元格禁则（待用户确认后执行）
  - [x] SubTask 0.7.1: `shape_cell` 改为禁则感知断行：本地按字形簇测宽 + 禁则边界选断点生成硬行，再以 `Wrap::None` 排版；`Wrap::WordOrGlyph` 在超窄列宽会退化为逐字形断行，不可用
  - [x] SubTask 0.7.2: 新增回归：窄列宽长 CJK 单元格的行首/行尾不出现禁则标点（覆盖 `》` 复现场景），并断言源码逐字节不变
  - [x] SubTask 0.7.3: 与 C5 表格交互一并真机复测；全量测试 / clippy / 体积门禁通过（探针 0 违规，见 `drive-v1-excellence` Task C1）

## 阶段 1：行盒影子对拍（触发式，暂不批准）

> **开工条件（至少满足一条）**：① 阶段 0 归因证明注入法无法修 C2；② 出现第 3 个无法在注入法下解决的行排需求；③ 用户明确要求推进。

- [ ] Task 1.1: 定义 `LineBox` 数据结构与不变量
  - [ ] SubTask 1.1.1: 字段：块内字节区间、行顶/行高、字形索引范围、悬挂标记与凸出量；文档注释写明与 cosmic `LayoutRun` 的对应关系
  - [ ] SubTask 1.1.2: 明确行高来源：逐行取字形 `line_height` 最大值，避免 fallback/emoji 行高跳动

- [ ] Task 1.2: 实现行盒计算（消费成形结果）
  - [ ] SubTask 1.2.1: 以 `Buffer::set_size(None, None)` 或公开 `ShapeLine`/`ShapeWord` 取得字形流，交由 `linebreak` 策略断点
  - [ ] SubTask 1.2.2: 行盒仅在测试或显式开关下计算；release 默认路径零开销

- [ ] Task 1.3: shadow 差异对拍
  - [ ] SubTask 1.3.1: 对拍旧 `layout_runs()` 视觉行边界与新行盒边界，输出差异报告
  - [ ] SubTask 1.3.2: 覆盖 10 万字语料、5005 块结构文档、`02-混合元素.md`、CJK 黄金语料
  - [ ] SubTask 1.3.3: 差异集必须为空；非空则本阶段不通过，不得进入阶段 2

- [ ] Task 1.4: 量化开销
  - [ ] SubTask 1.4.1: bench 记录行盒计算的额外耗时/内存，与基线对比
  - [ ] SubTask 1.4.2: 若开销侵蚀 p95 预算，结论记为“不建议进入阶段 2”，并保留阶段 0 成果

## 阶段 2：行盒成为几何真值（触发式，v1.1/v2 候选）

> **开工条件**：阶段 1 差异集为空 + 量化开销可接受 + 用户明确批准 + v1.0 tag 后。

- [ ] Task 2.1: 几何消费者迁移
  - [ ] SubTask 2.1.1: `hit` 改读行盒（x/y → 字节偏移），保留 y 越界钳位语义
  - [ ] SubTask 2.1.2: `caret_xy` 改读行盒，保留视觉行选择规则（折行不能落错行）
  - [ ] SubTask 2.1.3: `selection_rects` 改读行盒，覆盖 BiDi/样式切换多段与空行语义
  - [ ] SubTask 2.1.4: `line_tops`（打印分页）改读行盒
  - [ ] SubTask 2.1.5: 行内公式/图片定位改读行盒，保留基线对齐规则

- [ ] Task 2.2: 移除过渡补丁
  - [ ] SubTask 2.2.1: 删除 `build_typo_subs` 的 WJ 注入（断行由行盒策略直接决定）
  - [ ] SubTask 2.2.2: 删除 `hang_shift` 光栅位移，悬挂改由行盒悬挂标记表达
  - [ ] SubTask 2.2.3: 更新 `rasterize_buffer` 只消费行盒给出的最终字形位置

- [ ] Task 2.3: 缓存与性能
  - [ ] SubTask 2.3.1: `LayoutKey` 纳入行盒输入（策略版本、禁则表版本、版心），扩展失效范围
  - [ ] SubTask 2.3.2: bench 复测 p95 与内存，确认不回退
  - [ ] SubTask 2.3.3: 冷启动复测（≤300 ms 环境观察项口径不变）

- [ ] Task 2.4: 回归与范围声明
  - [ ] SubTask 2.4.1: 全量测试 + clippy 零警告 + release 构建
  - [ ] SubTask 2.4.2: 真机复测 C2/C5、D 区光标与选区、E 区打印分页
  - [ ] SubTask 2.4.3: 若 bidi/复杂脚本不在覆盖范围，在文档与代码注释中明确降级边界，不得静默退化

# Task Dependencies

- Task 0.2 依赖 Task 0.1（先有策略契约，再谈单一真值）
- Task 0.3 依赖 Task 0.1、Task 0.2
- Task 0.4 依赖 Task 0.1、Task 0.3
- Task 0.5 依赖 Task 0.4（先能自动判定，再上真机归因）
- Task 0.6 依赖 Task 0.1–0.5
- Task 1.x 依赖阶段 0 全部完成 + 阶段 1 触发条件成立
- Task 2.x 依赖阶段 1 门禁全部通过 + 用户批准
- Task 0.7 依赖 SubTask 0.5.4 的场景确认；未确认前只修表格路径
