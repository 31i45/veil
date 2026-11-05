# Checklist

> 阶段 0 已完成项已勾选；C2 真机复测与阶段 1/2 门禁未满足，保持未勾选。

## 阶段 0 验收

- [x] `crates/veil-ui/src/view/linebreak.rs` 存在，边界策略为纯函数：无 cosmic-text 依赖、无 IO、无全局状态，同一输入两次调用输出完全一致（实施修订：带 advance 的行走装配移入阶段 1）
- [x] 策略契约有文档注释，覆盖行首/行尾禁则集合、`glue_before/after`、胶水段生成与 UAX 覆盖边界
- [x] 行首/行尾禁则表已移出 `#[cfg(test)]`，实现与测试引用同一常量，代码中不存在第二份字符子集
- [x] `build_typo_subs` 的胶水段由策略产出，`Sub`/`OffsetMap` 结构未变；中西文间距逻辑保持不变且区间不重叠
- [x] 新增单测覆盖：行首禁则、行尾禁则、省略号/破折号粘连、UAX 覆盖字符不重复注入、文首/硬行首/行尾边界
- [x] CJK 禁则回归在 ±4px 版心变化下通过（真实字形度量，非人工估算）
- [x] 全量 `cargo test --workspace` 通过：900 passed / 12 ignored（新增 6 条）
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 零警告
- [x] release 构建成功，`veil.exe` 体积 5,844,480 B（≤15 MiB）
- [x] 击键延迟 bench 无回退：`keystroke_core` p95 1.07 ms <16 ms（bench_typing，5005 块，实测）
- [x] `crates/veil-core/**` 零改动；`veil-ui` 外部依赖仍为 4（未新增依赖）
- [x] C2 探针存在（`VEIL_BREAK_PROBE` 门控、默认关闭），输出每视觉行首/尾字符与违规行文本（cosmic 不暴露断点来源，暂无法输出）
- [x] C2 真机归因已完成并回填 `3e-walkthrough-results.md`：正文段落 240–800px + 深滚动 0 违规；表格单元格窄列宽复现 `》` 行首违规；状态保持「未解决」
- [ ] 表格单元格禁则修复（Task 0.7）完成并真机复测通过
- [x] 未在真机观察前把 C2 填为「通过」

## 阶段 1 门禁（满足后才可开工）

- [ ] 阶段 0 全部验收项通过
- [ ] 触发条件成立并记录：① 注入法被证明无法修 C2；或 ② 出现第 3 个无法在注入法下解决的行排需求；或 ③ 用户明确要求推进
- [ ] `LineBox` 结构与不变量有文档注释，字段覆盖字节区间、行顶/行高、字形索引范围、悬挂标记
- [ ] 行高逐行取字形最大值，已覆盖 fallback 字体/emoji 行高偏大的场景
- [ ] 行盒计算仅在测试或显式开关下运行，release 默认路径零额外开销
- [ ] shadow 差异报告覆盖 10 万字语料、5005 块结构文档、`02-混合元素.md`、CJK 黄金语料
- [ ] 旧 `layout_runs()` 与新行盒的视觉行边界差异集为空（非空即本阶段失败）
- [ ] 行盒计算开销已 bench 量化；若侵蚀 p95 预算，结论为「不进入阶段 2」

## 阶段 2 门禁（满足后才可开工）

- [ ] 阶段 1 全部门禁通过，且用户明确批准进入阶段 2
- [ ] `hit` / `caret_xy` / `selection_rects` / `line_tops` / 行内图定位全部以同一套 `LineBox` 为真值
- [ ] cosmic `Buffer::hit` 与 `Buffer::highlight` 不再参与块内几何计算
- [ ] WJ 注入与 `hang_shift` 光栅位移已删除；悬挂由行盒悬挂标记在几何层表达
- [ ] `LayoutKey` 已纳入行盒输入（策略版本、禁则表版本、版心），缓存失效范围覆盖
- [ ] 真机复测通过：C2/C5、折行处光标与拖选、双击/三击、Home/End、IME 候选框跟随、E 区打印分页（E3/E4/E7/E8）
- [ ] 折行、悬挂标点、行内公式/图片、CJK 混排场景下，光标落点、选区矩形、打印分页断点与渲染墨迹一致，无像素级错位
- [ ] bidi/复杂脚本的覆盖边界已在代码注释与文档中明确；若降级，不得静默发生
- [ ] 全量测试 + clippy 零警告 + release 构建通过；p95 与内存 bench 无回退；冷启动口径不变
- [ ] `crates/veil-core/**` 仍零改动

## 文档一致性

- [ ] 三阶段负面/受益分析已落档于 `spec.md` 的「三阶段影响与收益分析（决策用）」，且未出现未实测的性能承诺
- [ ] 本 spec 未修改任何项目台账以外的产品文档；代码零改动（提案阶段）
- [ ] 执行后若 C2/C5 状态变化，已同步 `docs/review/3e-walkthrough-results.md` 与 `.trae/specs/v1-progress-review-and-roadmap/review.md` 的四档规划对应行
