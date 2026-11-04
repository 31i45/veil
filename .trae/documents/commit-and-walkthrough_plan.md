# 提交修复 + 继续真机走查 Implementation Plan

## Repository Research

### 当前 git 状态

```
M  crates/veil-ui/src/document/block_index.rs   (1 line: prune → prune_alive)
M  crates/veil-ui/src/paint.rs                  (+11 lines: HashSet import + prune_viewport 调用)
M  crates/veil-ui/src/view/cache.rs             (+120 lines: prune_alive 重命名 + prune_viewport + 2 条新测试)
?? crates/veil/examples/gen_c10_md.rs           (10 万字 Markdown 生成器，C10 测试素材生产工具)
?? .trae/documents/viewport_cache_prune_plan.md  (修复实施计划，不入库——临时产物)
```

### 不入库的文件

- `.trae/documents/viewport_cache_prune_plan.md` — 本轮修复的实施计划，属于临时产物，不应入库。删除它。

### 不提交的文件

- `%TEMP%\veil-walkthrough\03-10万字.txt` — 10 万字 Markdown 测试素材（229KB），放在临时目录，不进 repo。

### 修复 commit 内容确认

**标题**：`fix: 视口驱动缓存回收，rasters/layouts/tables 滚动不再累积`

**body**：
- 根因：滚动帧不触发 `refresh_index → prune_alive`，`rasters`（≈500KB/块）/`layouts`/`tables` 三类 BlockId 索引 HashMap 只增不减。10 万字 Markdown（1625 Block）滚动到稳定后 PrivateWorkingSet = 367 MB（红线 150 MB）。
- 修复：cache 结构体新增 `prune_viewport(keep_ids, base)` 方法，只剪 raster/layout/table；paint 帧末尾、frame 送还之前，构造 `keep_ids = visible_index_range() + caret 块 + math_imgs 键` 并调 prune_viewport。base_key 不匹配的条目双重保险剪（resize/DPI 风暴期间旧键残留）。
- 重命名：原 `prune()` 重命名为 `prune_alive()`（语义明确为"索引重建驱动的全量回收"）。
- 实测：PrivateWorkingSet 从 **367 MB → 28.83 MB**（↓ 92%），稳定在 28-29 MB。
- 新增回归测试：`prune_viewport_reclaims_non_visible_rasters`（非可见块被剪 + base_key 双重保险）、`prune_viewport_leaves_content_addressed_caches_alone`（内容寻址缓存不动）。
- 门禁：553 passed / 0 failed / 11 ignored；clippy 零警告；release 5.56 MiB（零增长）。
- 测试素材更新：纯 txt 单块 `03-10万字.txt` → 10 万字 Markdown（1625 Block / 3575 行），含标题/段落/列表/表格/代码块/引用。

### 真机走查进度

**已完成 13/65 项**（A1-A6 + B1-B7）。本轮 C10 也已通过（28.83 MB + 秒开 + 滚动跟手 + 无降级提示）。

**剩余 51 项 + 2 复验项**：

| 优先级 | 数量 | 明细 | 是否阻断 tag |
|---|---|---|---|
| **P0 阻断** | 1 | **E3 多页打印 PDF 目视** | ✅ 是 |
| P1 高 | 38 | C1-C9, C11, D1, D3-D5, D7-D12, D14, E4, E7-E8, F1-F3, G1-G2, G9, H1, H3-H4 | ⚠️ 不通过则立项修复 |
| P2 中 | 6 | D2, D6, D13, E1, E2, F6 | ❌ 降级可接受 |
| P3 低 | 4 | H2（v2 立项）, E5, E6, F4 | ❌ 已决策降级 |
| 复验 | 2 | A1（标题栏图标）、K1（Ctrl 组合键） | 已修需真机确认 |

### 走查顺序策略

**阶段 1：P0 阻断**（先过红线，否则测了白测）
- E3 多页打印 → Microsoft Print to PDF → 打开 PDF 目视有无重字/截字

**阶段 2：P1 高优先级批量**（C→D→E→F→G→H，按区集中）
- C 区剩余 10 项（C1-C9, C11）— 查找栏/菜单/快捷键
- D 区剩余 10 项（D1, D3-D5, D7-D12, D14）— DPI/滚动/窗口
- E 区剩余 3 项（E4, E7-E8）— 打印路径（P0 过了再测 E4/E7/E8）
- F 区剩余 3 项（F1-F3）— 图片独占段/快捷键
- G 区 3 项（G1-G2, G9）— 进程级场景（多实例/恢复）
- H 区 3 项（H1, H3-H4）— 大纲/右键菜单

**阶段 3：复验项 + P2/P3 降级处理**
- A1 标题栏图标（已修 d013e2f）
- K1 Ctrl 组合键（已修 d85e278）
- P2/P3 按分区表降级

### 走查前置条件

1. **冷环境**：`%APPDATA%\Veil\` 已备份走（上一步已做），每次开 veil 都是干净状态
2. **release 产物**：`target\release\veil.exe`（已构建，5.56 MiB）
3. **测试素材**：`%TEMP%\veil-walkthrough\` 下 5 份（01 短文 / 02 混合元素 / 03 10 万字 / 04 大表格 / 05 数学）
4. **走查记录**：`docs/review/3e-walkthrough-results.md` 用 ✅ / ❌ / ⚠️ 回填

---

## Files and Modules

### 提交阶段

| 文件 | 操作 | 说明 |
|---|---|---|
| `crates/veil-ui/src/document/block_index.rs` | stage + commit | 1 行改名 |
| `crates/veil-ui/src/paint.rs` | stage + commit | 11 行新增 |
| `crates/veil-ui/src/view/cache.rs` | stage + commit | 120 行新增/改名/测试 |
| `crates/veil/examples/gen_c10_md.rs` | stage + commit | 新增，C10 素材生产工具 |
| `.trae/documents/viewport_cache_prune_plan.md` | **删除** | 临时产物，不入库 |

### 走查阶段

| 文件 | 操作 | 说明 |
|---|---|---|
| `docs/review/3e-walkthrough-results.md` | 边测边回填 | ✅ / ❌ / ⚠️ 标记 |
| `target\release\veil.exe` | 运行 | release 产物 |
| `%TEMP%\veil-walkthrough\*` | 运行时用 | 5 份测试文档 |

---

## Implementation Steps

### Part 1：提交修复

1. **删除临时 plan 文件**：`.trae/documents/viewport_cache_prune_plan.md`
2. **确认工作区干净度**：`git status --short` 只剩 4 个文件待 stage（3 modified + 1 untracked）
3. **全量门禁再跑一遍**：`cargo test --workspace` + `cargo clippy` — 提交前最后验证
4. **git add + commit**：
   ```
   git add crates/veil-ui/src/document/block_index.rs \
            crates/veil-ui/src/paint.rs \
            crates/veil-ui/src/view/cache.rs \
            crates/veil/examples/gen_c10_md.rs
   git commit -m "fix: 视口驱动缓存回收，rasters/layouts/tables 滚动不再累积

   根因: 滚动帧不触发 refresh_index→prune_alive, BlockId 索引 HashMap
   只增不减, 10 万字 Markdown(1625 Block) 滚动后 PrivateWorkingSet=367MB.
   
   修复: Cache 新增 prune_viewport(keep_ids, base) 只剪 raster/layout/table;
   paint() 帧末尾构造 keep_ids=visible_index_range()+caret+math_imgs 键.
   
   结果: PrivateWorkingSet 367MB→28.83MB (↓92%), 稳定不回落.
   
   新增回归测试 prune_viewport_reclaims_non_visible_rasters +
   prune_viewport_leaves_content_addressed_caches_alone."
   ```
5. **验证 commit**：`git show --stat HEAD` + `git status --short`（必须干净）

### Part 2：继续真机走查

按优先级分区逐阶段推进，每完成一组就回填 `3e-walkthrough-results.md`：

**阶段 1：E3 多页打印（P0 阻断）**
1. 启动 release veil
2. 打开 `02-混合元素.md`
3. 文件 → 打印 → 选择 Microsoft Print to PDF → 保存
4. 用 PDF 阅读器打开，逐页目视：有无重字？截字？分页断点正确？
5. 回填 E3 → pass 后进入阶段 2

**阶段 2：P1 高优先级批量（按 C→D→E→F→G→H 顺序）**
每区集中测，测完回填整区：
- C 区（C1-C9, C11）：查找栏可达/快捷键/菜单开着的 Ctrl 组合
- D 区（D1, D3-D5, D7-D12, D14）：150% DPI/滚动平滑/窗口缩放/恢复
- E 区（E4, E7-E8）：打印对话框/EPSON 驱动/打印撤销
- F 区（F1-F3）：图片独占段/Ctrl+Shift+I 粘贴图片
- G 区（G1-G2, G9）：多实例互斥/恢复旧文档
- H 区（H1, H3-H4）：大纲/右键菜单

**阶段 3：复验 + P2/P3 降级**
- A1：标题栏图标（d013e2f 已修）
- K1：Ctrl 组合键（d85e278 已修）
- P2 6 项：确认降级
- P3 4 项：确认降级

**阶段 4：走查收尾**
- 汇总所有结果到 `3e-walkthrough-results.md`
- 判断是否还有阻断 tag 的未通过项
- 决定是否可以 `git tag -a v1.0.0`

---

## Dependencies and Considerations

### 提交阶段

- **git 规范**：仓库用中文 commit message（已有先例：`v1 决策收口（文档）`、`fix: Resolved 0xC0000409 crash`）
- **LF/CRLF**：Windows 下 Read 读到 CRLF、Edit 后写回 CRLF，git 会提示 `LF will be replaced by CRLF` warning，可忽略（不是实际差异）
- **gen_c10_md.rs 入库**：这个 example 是 C10 测试素材生产工具，入库是为了以后需要再生成大文档时有现成工具。不改代码逻辑（不影响产品）。

### 走查阶段

- **冷环境**：`%APPDATA%\Veil\` 已备份走（`C:\Users\31i45\AppData\Local\Temp\veil-appdata-backup-20261104-112735`），每次开 veil 都是干净状态。如果中途恢复了旧备份，需要再次移走。
- **release 产物必须是本次修复后的**：`target\release\veil.exe` 已在 Step 5 重建完成（2026/11/4 14:57:16），5.56 MiB。
- **走查时间估算**：P0 E3（5-10min）+ P1 38 项（每区 15-30min，共 90-180min）+ 复验 P2/P3（10min）≈ 约 2-3 小时。**不需要一次跑完**——测完一个区可以停，回填记录后下次继续。
- **遇到问题**：走查过程中发现缺陷，先记录到 `3e-walkthrough-results.md` 的「发现」列，**不要在走查期间修代码**——本轮目标是"摸清现状"，缺陷统一汇总后再决定是否修复。
- **键盘输入语言**：走查快捷键相关项时，**切换到英文输入法**（中文输入法下某些 Ctrl 组合键被输入法拦截）。
- **DPI**：当前环境是 150%（GetDpiForWindow=144），符合走查清单的 150% 测试条件。

---

## Validation

### 提交阶段

| 检查 | 方法 | 通过标准 |
|---|---|---|
| 全量测试 | `cargo test --workspace` | 553 passed / 0 failed |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| commit | `git show --stat HEAD` | 4 文件 / +125 行 / -8 行 |
| 干净树 | `git status --short` | 空 |
| 临时文件 | `.trae/documents/viewport_cache_prune_plan.md` | 不存在 |

### 走查阶段

| 检查 | 方法 | 通过标准 |
|---|---|---|
| E3 PDF | 目视每一页 | 无重字 / 无截字 / 分页断点正确 |
| C1-C11 | 按清单逐条操作 | 每项 ✅ / ❌ 标记 |
| DPI 视觉 | 150% DPI 下字体/图标清晰 | 无模糊/重影/错位 |
| 滚动跟手 | 拖滚动条 + 滚轮 | 完全跟手，无卡顿 |
| 内存稳定 | 任务管理器 PrivateWorkingSet | ≤ 150 MB 稳定 |
| 记录完整 | `3e-walkthrough-results.md` | 所有已测项都有 ✅/❌ 结论 |

---

## Risks

| 风险 | 概率 | 影响 | 处理 |
|---|---|---|---|
| 某个 P1 项实际是缺陷 | 中 | 需要立项修复，tag 延后 | 记录在案，不阻塞其他项，走查完统一评估 |
| E3 打印路径有真实缺陷 | 低 | 阻断 tag | 走查期间记录，优先修复（打印是发布门槛） |
| 冷环境意外恢复 | 极低 | G/A 区测试结论被旧 session 污染 | 开始测 G 区前再次确认 `%APPDATA%\Veil\` 不存在 |
| 走查中断下次继续 | 必然 | 不影响 | `3e-walkthrough-results.md` 实时回填，随时可从上次停止处继续 |
| `target\release\veil.exe` 被后续 cargo clean 清掉 | 低 | 需要重新构建（约 2min） | 构建已完成，走查期间不要跑 `cargo clean`；如果清了就重 build |
| 键盘输入法干扰 | 中 | 快捷键测试结论不准 | 测快捷键前先切英文输入法 |
