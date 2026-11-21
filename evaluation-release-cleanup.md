# evaluation-release-cleanup.md（Task 1 清理执行记录）

> 状态：**已执行清理；未 commit、未 push**。本文件由 Task 0 审计草稿更新为 Task 1 实测结果，草稿中的估算已替换为实测值。
> 执行时间：2026-11-21。基线提交：`79a8079 docs(ux): bind final evidence to the shipped binary`。

## 0. 最终发布产物与门禁（2026-11-21 收口）

| 项 | 结果 |
|---|---|
| 产物 | `veil.exe` **14,012,928 B = 13.36 MiB**（门禁 ≤14 MiB = 14,680,064 B，余量 667,136 B） |
| SHA-256 | **`ff23c4d9621d586e8edcee90fb7d573c6df0665b98c4ae4a32fdfb974ddd25d0`**（`FF23C4D9…DD25D0`） |
| 源码 | commit `0d229eb` |
| 门禁 | fmt、workspace release 测试（`cargo test --workspace --release -- --test-threads=1`）、clippy、size、feature（`QuickJS` / `linkedom` / `mermaid` 命中）、smoke、checksum **全部 passed**；`release_ready=true`，机器可读记录 `evidence/release/release-record.json` |
| 归档副本 | `evidence/release/{release-record.json,veil.exe.sha256}` 已用 `target/release/` 同名文件覆盖，SHA-256 复核与最终产物一致；`release-bundle/veil-v3.0.0/veil.exe.sha256` 同为该值 |
| 与 UX 证据 | `evaluation-ux.md` 绑定的 `8DF551F8…9B456763` 是同源码、同字节数的前一构建（差异仅 PE 时间戳），**不是最终发布包**；UX 数字在该源码上测得，不失效 |
| `.trae` 限制 | `.trae/` 57.95 MB / 835 文件未清理：删除包装器对 `.trae` 全目录设 denylist，操作被拒；spec 文档与 evidence 原样保留（见 §4） |
| 推送状态 | **PENDING**（本轮未 commit、未 push） |

## 1. 清理前后实测（体积 / 文件数）

| 目录 | 清理前 | 清理后 | 说明 |
|---|---:|---:|---|
| `target/` | 6,193.72 MB / 7,500 | 已删除 | 构建缓存，gitignored |
| `target-default/` | 1,693.62 MB / 3,281 | 已删除 | 旧默认 feature 构建缓存，gitignored |
| `spikes/` | 15.18 MB / 302 | 已删除 | gitignored 原型目录（引用风险见 §5 R6） |
| `.screenshots/` | 0.00 MB / 0 | 已删除 | 仅空 `.tmp` 目录 |
| `release-bundle/` | 16.20 MB / 8 | **13.48 MB / 7** | 仅删 `veil.pdb`（2.72 MiB） |
| `evidence/` | 653.65 MB / 4,528 | **312.91 MB / 1,813** | 明细见 §2 |
| `.trae/` | 57.95 MB / 835 | **57.95 MB / 835** | 未改动（§4） |
| `bench/` | 0.00 MB / 0 | 0.00 MB / 0 | 空目录 |
| **合计** | **8,630.32 MB / 16,446** | **384.34 MB / 2,655** | 工作区减少 **8,245.98 MB（≈8.05 GB）/ 13,791 文件** |

`evidence/` 清理后分项：`ux/` 70.51 MB / 1,029、`render/` 237.07 MB / 448、`startup/` 5.07 MB / 302，其余 `ci`、`decision`、`evaluation`、`g0`、`input`、`mermaid`、`not-run`、`print`、`release` 合计 0.23 MB / 28。

`target/` 重建后实测 **4,453 文件 / 1,819.63 MB ≈ 1.8 GB**（release 构建 + 依赖与增量缓存），仍远小于清理前的 6,193.72 MB；`target/` 为可再生产物，不入库。最终产物 SHA-256 见 §0。

## 2. 已删除明细（实测）

| # | 目标 | 实测删除 |
|---|---|---:|
| D1 | `target/` | 6,193.72 MB / 7,500 |
| D2 | `target-default/` | 1,693.62 MB / 3,281 |
| D3 | `spikes/` | 15.18 MB / 302 |
| D7 | `evidence/real-machine-2026-11-12/`（grep 确认无文档引用） | 76.28 MB / 1,530 |
| D8 | `release-bundle/veil-v3.0.0/veil.pdb` | 2.72 MiB / 1 |
| D9 | `evidence/ux/**/_work/`、`_home/`（先 `git rm -r --cached`） | 479 文件（其中 tracked 125）/ <1 MB |
| D12 | `.screenshots/` | 0 文件 |

历史轮次按「保留 md/json/txt 摘要、只删大截图」执行：

| 轮次 | 删除内容 | 实测 |
|---|---|---:|
| `ux/final` / `final-v2` / `final-v3` / `final-v4` | `*.png`、`*.pdf`（含各 26.5 MB 的 `corpus.pdf`） | 554 文件 / 253.97 MB |
| `ux/after-v2`…`after-v6` | `*.png`（`screenshots/` 实拍） | 166 文件 / 9.89 MB |

保留的摘要：各轮 `*.md`、`*.json`、`*.csv`、`*.txt`、`*.log`、`*.html`、`*.ps1` 全部留存；`final-v3/perf/run2.log`（`CHANGELOG.md` L43 直接引用）**保留原路径未动**。

## 3. 必须保留清单与存在性抽查

保留：`evidence/ux/{baseline,after,after-v7,final-v5}`、`evidence/{evaluation,render/task8a-b,render/task9-final,render/task8a/rubric.md,render/rubric-final.md,startup,input,g0,ci,not-run,decision,mermaid,print,release}`、根目录三份 `*-task9-final.md`、`release-bundle/veil-v3.0.0`（`veil.exe` 14,012,928 B、`veil.exe.sha256`、`release-record.json`、文档、图标）。

抽查（12 条，全部存在）：

| 路径 | 结果 |
|---|---|
| `evidence/ux/after-v7/README.md` | 存在 |
| `evidence/ux/final-v5/report.md` | 存在 |
| `evidence/ux/final-v3/perf/run2.log` | 存在（CHANGELOG L43 引用） |
| `evidence/ux/baseline/facts.md` | 存在 |
| `evidence/render/task8a-b/rubric.md` | 存在 |
| `evidence/render/task9-final/summary.md` | 存在 |
| `evidence/render/task8a/rubric.md` | 存在（evaluation.md L169 引用） |
| `evidence/startup/task9-final/d1/summary.md` | 存在 |
| `evidence/release/v3-build.md` | 存在 |
| `evidence/release/release-record.json` | 存在（已被最终构建输出覆盖，文件 SHA-256 `272EDA49…427B4C`，与 `release-bundle` 副本逐字节一致） |
| `release-bundle/veil-v3.0.0/veil.exe` | 存在（14,012,928 B，SHA-256 `ff23c4d9…dd25d0`） |
| `release-bundle/veil-v3.0.0/veil.exe.sha256` | 存在（内容 `ff23c4d9…dd25d0`，文件 SHA-256 `783DCAF9…438A96`） |

## 4. 未执行 / 受阻项

| 项 | 原因 |
|---|---|
| `.trae/specs/**/evidence/` 下截图（约 52 MB） | 本机删除包装器对 `.trae` 全目录设 denylist，操作被拒；**spec 文档与 evidence 均原样保留**。 |
| `evidence/render/{dark,light}`、`evidence/render/task8a/{dark,light}`（≈122 MB） | 仅被 `evaluation.md` L4 以范围名 `evidence/render` 泛引用；不在明确授权清单内，未删除，避免引用范围名不副实。 |
| `evidence/ux/final*` 的 PDF 已删 | 与该目录既有「历史证据原样保留」表述不完全一致，属本次清理的授权例外。 |

## 5. 引用风险

| ID | 风险 | 状态 |
|---|---|---|
| R1 | `CHANGELOG.md` L43 引用 `evidence/ux/final-v3/perf/run2.log` | **已规避**：文件保留原路径。 |
| R2 | `CHANGELOG.md` L46、`evaluation-ux.md` L7–L12/L134–L135/L206 引用 `after-v2…v6`、`final…final-v4` | **部分降级**：目录与 md/json/txt 摘要仍在，实拍 PNG/PDF 已删；文档未改写（不 commit）。 |
| R3 | `evaluation.md` L4/L13、`CHANGELOG.md` L47/L48、`README*.md` 引用 `target/release/release-record.json` | **已修复**：统一改指 `evidence/release/release-record.json`（归档副本，字节一致），并注明 `target/` 为可再生产物。 |
| R4 | `evaluation.md` L4 泛引用 `evidence/render` | 未触发（render 子目录保留）。 |
| R5 | README L149 smoke 示例 `target/release/veil.exe` | 路径暂不存在，需重建；命令模板语义不变。 |
| R6 | `CHANGELOG.md` L108、`docs/adr/ADR-002/011/012`、`docs/review/component-replacement-spike.md`、`docs/01-define-design.md` 引用 `spikes/...`（`RESULTS.md`、`samples.md`、`src/bin` 等） | **已修复**：不恢复目录，全部改为「本地原型 `<name>`（已清理）」并保留结论/数字；`spikes/` 仅作为 `.gitignore` 规则名与清理记录出现，不再指向具体文件。 |
| R7 | `evidence/ux/final*` 的 `corpus.pdf`（每份 26.5 MB）随 D5 删除 | 已回收 211.9 MB；`final-v5` 的两份保留。 |

## 6. Git 状态（清理后）

```
$ git status --short | wc -l
128
# 125 个暂存删除（D，来自 git rm -r --cached 的 _work/_home）
# 3 个未跟踪文档：docs/manual-test-veil.md、docs/perfect-writing-surface-strategy.md、evaluation-release-cleanup.md

$ git ls-files evidence | Select-String _work|_home
# 无输出
```

- **推送状态 PENDING**：未 commit、未 push；本地 `main` 仍领先 `origin/main` 18 个提交（文档/清理提交尚未产生）。
- `evidence/` 仅 `evidence/ux/` 下文件被跟踪，其余为本地未跟踪证据。

## 7. 磁盘空间说明

- 工作区目录实测减少 **8,245.98 MB ≈ 8.05 GB**。
- 本机删除动作经安全包装器转入 D 盘回收站：清理后 `D:\$Recycle.Bin` 占用 **8.77 GB**，`D:` 可用空间维持 **429.78 GB** 未变。**需用户清空回收站后，约 8 GB 才真正释放**；代理无回收站访问/清空权限。

## 8. 证据保留规则（长期）

适用对象：`evidence/**`、`release-bundle/**`、根目录 `evaluation*.md`、`README*.md`、`CHANGELOG.md`。

1. **被引用即保留**：任何被 `evaluation*.md`、`README*.md`、`CHANGELOG.md`、`docs/**` 以具体路径引用的证据（摘要 md/json/csv/txt/log、二进制、截图）必须保留；若要删除，先同步改引用或降级为结论描述。
2. **中间构建证据可降级**：`_work/`、`_home/`、临时截图/日志、`*.tmp`、中间构建产物（`target/`、`target-default/`、`spikes/`、`release-bundle/**/*.pdb`）不入库；在下一轮最终证据产生后，可删除上一轮的二进制/大图，仅保留 md/json/txt/log 摘要。
3. **最终证据摘要不动**：`evidence/ux/{baseline,after,after-v7,final-v5}`、`evidence/release/`、`evidence/{render/task9-final,startup/task9-final,input,g0,ci,not-run,decision,mermaid,print}` 等被最终报告引用的摘要继续跟踪，不因清理取消跟踪。
4. **删除前必须 grep 引用**：`spikes/`、`target/release/release-record.json`、`*.png`、`*.pdf` 等删除前，先全仓库 grep 路径；有引用则先改文档再删除，确保 0 悬空引用。
5. **路径口径**：发布记录统一引用 `evidence/release/release-record.json`（归档副本）/ `evidence/release/veil.exe.sha256`；`target/release/` 下的同名文件属可再生产物，不作为文档真值。

Task 2 复验：`git status --short` 无 `_work/`、`_home/` 跟踪（仅保留 125 个既有暂存删除）；全仓库 grep `spikes/` 仅命中 `.gitignore` 规则与本文清理记录，无悬空文件引用。
