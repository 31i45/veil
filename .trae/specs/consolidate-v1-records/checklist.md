# Checklist

- [x] 每个走查项在 `3e-walkthrough-results.md` 中只有一个权威状态，且四类口径（已通过 / 已修复 / 未解决 / 未测）无遗漏
- [x] 已修复项均标注证据（测试名或 commit hash），未解决项均标注现象与下一步
- [x] 未处理清单覆盖 D12-b、C5、C2、C7、内核三条，每项含「现象 / 嫌疑或根因 / 下一步 / 归属」
- [x] D12 的旧「已修复」结论已移除，改为 D12-b（未解决）+「仅编辑路径正确」子结论
- [x] 版本规划为四档（v1.0 门禁 / v1.1 / 批次 3 / v2），每档标注来源与触发条件，无重复立项
- [x] `3e-manual-walkthrough.md` 的 D12 行与台账一致（未解决）
- [x] `project_memory.md` 快照与实际基线一致：894 passed / 12 ignored、release 5,840,896 B、commit 至 `4deeca9`
- [x] 全库无残留过期结论（D12 已修复、旧测试数/体积）；历史章节统一加「历史快照」标注，当前口径为 894/12/5,840,896 B
- [x] `git status` 仅含 `docs/review/*`、`.trae/specs/*` 等文档变更（含 README 数字同步），`crates/**` 零改动
