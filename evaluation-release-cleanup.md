# evaluation-release-cleanup.md（Task 7.1 复核报告）

> 状态：**已清理、已通过门禁、已推送**。本文件已随 `4271228` 提交并推送到 `origin/main`。
> 复核日期：2026-11-21。基线提交：`79a8079 docs(ux): bind final evidence to the shipped binary`；清理提交：`e9b2256 chore: clean repository, archive release evidence, add manual QA and strategy`；报告提交：`4271228 docs: finalize repository cleanup report`。

## 0. 结论

**`PASS WITH LIMITS`**。可控项（清理、证据保留、门禁、推送）全部有实测证据；LIMITS 仅限以下三类，均不属于本轮可自动闭环项：

| LIMITS | 内容 |
|---|---|
| L1 外部人工项 | 真实中文 IME、100% / 125% / 200% DPI、实体打印、真实 GitHub Actions、Linux 桌面、真实断网抓包 —— **NOT RUN**，协议见 §7，不得写成 PASS（§8）。 |
| L2 `.trae` 未清理 | `.trae/specs/**/evidence/` 截图约 52 MB 未删：删除包装器对 `.trae` 全目录设 denylist，操作被拒；spec 文档与证据原样保留（§3、§4）。 |
| L3 API 边界 | 用户拖拽与 config 恢复路径已精确钳制最小窗口 520×360；程序化 `SetWindowPos` 仍可绕过（OS API 层，非用户路径），见 `evaluation-ux.md` §5.1 裁决“PASS（已知边界）”。 |

**纪律**：未执行的验证一律保持 `NOT RUN`；本报告与 `evaluation.md`、`evaluation-ux.md` 均未把 NOT RUN 写成 PASS。

## 1. 最终发布产物

| 项 | 结果 |
|---|---|
| 产物 | `release-bundle/veil-v3.0.0/veil.exe` **14,012,928 B = 13.36 MiB**（门禁上限 14,680,064 B = 14 MiB，余量 667,136 B） |
| SHA-256 | **`ff23c4d9621d586e8edcee90fb7d573c6df0665b98c4ae4a32fdfb974ddd25d0`** |
| 源码 | `0d229eb fix(ux): make every failure notice actionable`；`e9b2256` 仅新增文档/清理（未触及 `crates/`、`Cargo.*`、`tools/`），不改变二进制 |
| 门禁 | fmt、workspace release 测试、clippy、size、feature、smoke、checksum **全部 passed**；`release_ready=true`（明细见 §5） |
| 机器可读记录 | `evidence/release/release-record.json`（`status=passed`，`release_ready=true`），文件 SHA-256 `272EDA49…427B4C` |
| 归档一致性 | `evidence/release/release-record.json` 与 `release-bundle/veil-v3.0.0/release-record.json` 逐字节一致；两份 `veil.exe.sha256` 内容均为 `ff23c4d9…dd25d0`（文件 SHA-256 `783DCAF9…438A96`） |
| 与 UX 证据的关系 | `evaluation-ux.md` 绑定的 `8DF551F8D363C7…9B456763` 是与最终包**同源码、同字节数**的前一构建（仅 PE 时间戳不同），UX 数字仍有效；更早的 `97d3a60 / 58c9ad0d… / 13,998,592 B` 见 `evaluation.md` §1，为历史构建记录 |

## 2. 清理前后实测（体积 / 文件数）

| 目录 | 清理前 | 清理后（重建前） | 当前 | 说明 |
|---|---:|---:|---:|---|
| `target/` | 6,193.72 MB / 7,500 | 已删除 | **不存在** | 构建缓存；重建 4,453 文件 / 1,819.63 MB 后，推送完成执行 `cargo clean` 删除 |
| `target-default/` | 1,693.62 MB / 3,281 | 已删除 | **不存在** | 旧默认 feature 构建缓存 |
| `spikes/` | 15.18 MB / 302 | 已删除 | **不存在** | 原型目录（引用已改写，见 §9 R6） |
| `.screenshots/` | 0.00 MB / 0 | 已删除 | **不存在** | 仅空 `.tmp` 目录 |
| `release-bundle/` | 16.20 MB / 8 | 13.48 MB / 7 | **16.20 MB / 8** | 清理时删 `veil.pdb`；最终归档再次复制 `veil.exe`、`veil.pdb`、记录与文档 |
| `evidence/` | 653.65 MB / 4,528 | 312.91 MB / 1,813 | **312.91 MB / 1,813** | 明细见下 |
| `.trae/` | 57.95 MB / 835 | 57.95 MB / 835 | **57.95 MB / 835** | 未改动（denylist，L2） |
| `bench/` | 0.00 MB / 0 | 0.00 MB / 0 | **0.00 MB / 0** | 空目录 |
| **合计** | **8,630.32 MB / 16,446** | **384.34 MB / 2,655** | **387.06 MB / 2,656** | 较清理前减少 **8,243.26 MB（≈8.05 GB）/ 13,790 文件** |

- `evidence/` 当前分项：`render/` 237.07 MB / 448、`ux/` 70.51 MB / 1,029、`startup/` 5.07 MB / 302，其余 `ci`、`decision`、`evaluation`、`g0`、`input`、`mermaid`、`not-run`、`print`、`release` 与根级 6 个文件合计 0.25 MB / 34。
- `target/` 重建实测 4,453 文件 / 1,819.63 MB ≈ 1.8 GB（release 构建 + 依赖与增量缓存），仍小于清理前 6,193.72 MB；`target/` 为可再生产物，不存档、不入库。
- `release-bundle/` 当前 8 个文件：`veil.exe`（14,012,928 B）、`veil.pdb`（2.72 MiB）、`veil.exe.sha256`、`release-record.json`、`README.md`、`CHANGELOG.md`、`LICENSE`、`icons/veil.ico`。`veil.pdb` 是最终归档步骤重新生成的调试符号，不影响 `veil.exe` 与 SHA；如需回到 13.48 MB，可单独删除该 pdb。

## 3. 已删除清单（实测）

| # | 目标 | 实测删除 |
|---|---|---:|
| D1 | `target/` | 6,193.72 MB / 7,500 |
| D2 | `target-default/` | 1,693.62 MB / 3,281 |
| D3 | `spikes/` | 15.18 MB / 302 |
| D7 | `evidence/real-machine-2026-11-12/`（grep 确认无文档引用） | 76.28 MB / 1,530 |
| D8 | `release-bundle/veil-v3.0.0/veil.pdb`（清理时；最终归档已重新生成，见 §2） | 2.72 MiB / 1 |
| D9 | `evidence/ux/**/_work/`、`_home/`（先 `git rm -r --cached`） | 479 文件（tracked 125）/ <1 MB |
| D12 | `.screenshots/` | 0 文件 |

历史轮次按“保留 md/json/txt 摘要、只删大截图”执行：

| 轮次 | 删除内容 | 实测 |
|---|---|---:|
| `ux/final` / `final-v2` / `final-v3` / `final-v4` | `*.png`、`*.pdf`（含各 26.5 MB 的 `corpus.pdf`） | 554 文件 / 253.97 MB |
| `ux/after-v2`…`after-v6` | `*.png`（`screenshots/` 实拍） | 166 文件 / 9.89 MB |

保留的各轮摘要：`*.md`、`*.json`、`*.csv`、`*.txt`、`*.log`、`*.html`、`*.ps1` 全部留存；`final-v3/perf/run2.log`（`CHANGELOG.md` L43 引用）**原路径保留未动**。

## 4. 保留清单与证据引用抽检

保留：`evidence/ux/{baseline,after,after-v7,final-v5}`（另保留 `after-v2…v6`、`final…final-v4` 的摘要文件）、`evidence/{evaluation,render/task8a-b,render/task9-final,render/task8a/rubric.md,render/rubric-final.md,startup,input,g0,ci,not-run,decision,mermaid,print,release}`、`evidence/` 根级三份 `*-task9-final.md`（`actions-linux-needs-user`、`needs-user-dpi-100`、`print-manual`，`evaluation.md` L20 引用）、`release-bundle/veil-v3.0.0` 全部 8 个文件。`evidence/render/{dark,light,task8a/dark,light}`（≈122 MB）因 `evaluation.md` L4 以范围名 `evidence/render` 引用而保留。

抽检 12 条（逐条 `Test-Path`，全部为 `True`）：

| 路径 | 引用出处 | 结果 |
|---|---|---|
| `evidence/ux/after-v7/README.md` | `evaluation-ux.md` L7–L8、L13、L44 | 存在 |
| `evidence/ux/final-v5/report.md` | `evaluation-ux.md` L7–L8、L13 | 存在 |
| `evidence/ux/final-v3/perf/run2.log` | `CHANGELOG.md` L43 | 存在 |
| `evidence/ux/baseline/facts.md` | `evaluation-ux.md` L7（`evidence/ux/baseline` 目录级） | 存在 |
| `evidence/render/task8a-b/rubric.md` | `evaluation.md` L119、L135、L218 | 存在 |
| `evidence/render/task9-final/summary.md` | `evaluation.md` L17、L194 | 存在 |
| `evidence/render/task8a/rubric.md` | `evaluation.md` L169 | 存在 |
| `evidence/startup/task9-final/d1/summary.md` | `evaluation.md` L14、L31、L68、L234 | 存在 |
| `evidence/release/v3-build.md` | `CHANGELOG.md` L48 | 存在 |
| `evidence/release/release-record.json` | `evaluation.md` L4、L13、L18 | 存在（SHA-256 `272EDA49…427B4C`，与 `release-bundle` 副本逐字节一致） |
| `release-bundle/veil-v3.0.0/veil.exe` | `evidence/release/release-record.json` `archive.path`；spec `release-and-roadmap` TR-4.3（归档必须含 `veil.exe`） | 存在（14,012,928 B，SHA-256 `ff23c4d9…dd25d0`） |
| `release-bundle/veil-v3.0.0/veil.exe.sha256` | `release-record.json` `checksum.bundled_sha256` | 存在（内容 `ff23c4d9…dd25d0`，文件 SHA-256 `783DCAF9…438A96`） |

## 5. 门禁执行明细（推送前）

| 门禁 | 命令 / 依据 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | **passed** |
| 测试 | `cargo test --workspace --release -- --test-threads=1` | **passed**（`release-record.json` `tests.status=passed`，exit 0） |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **passed** |
| 体积 | 14,012,928 B ≤ 14,680,064 B | **passed**，余量 667,136 B |
| 特性 | 对 `veil.exe` 字节做 ASCII 解码 | **passed**，`QuickJS` / `linkedom` / `mermaid` 均命中 |
| 冒烟 | `pwsh -NoProfile -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000` | **passed**，exit 0 |
| 校验和 | 归档前后 SHA-256 复核 | **passed**，`post_archive_sha256` = `bundled_sha256` = `ff23c4d9…dd25d0` |
| 汇总 | `evidence/release/release-record.json` | `status=passed`、`release_ready=true`、`error=null` |

## 6. 推送状态（已完成）

```
$ git push origin main
   0b5092b..e9b2256  main -> main

$ git status -sb
## main...origin/main

$ git rev-parse HEAD; git rev-parse origin/main
e9b2256818c4d4024bc5e8b87543593b9035a189
e9b2256818c4d4024bc5e8b87543593b9035a189

$ git status --porcelain | Measure-Object   # 推送后、本次复核前
Count: 0
```

- 推送范围 `0b5092b..e9b2256` 共 **19 个提交**；本地 `main` 与 `origin/main` 指向同一提交。
- 未 force push、未改写历史；`git status -sb` 无 ahead/behind 标记。
- 推送后执行 `cargo clean`，`target/`、`target-default/`、`.screenshots/`、`spikes/` 已确认不存在。
- 本次 Task 7 报告已随 `4271228` 提交并推送；复核结束时工作树干净。

## 7. 人工测试协议与策略文档（已交付）

| 交付物 | 位置 | 内容指向 |
|---|---|---|
| 人工测试协议 | `docs/manual-test-veil.md`（commit `e9b2256`） | §A 首跑、§B 输入保真（B2 微软拼音 IME）、§C 文件、§D 导航、§E 窗口（E4 四档 DPI）、§F 导出（F3 实体打印）、§G 信任：零出站验证（G1 SniffNet、G2 Wireshark，附 `pktmon` / `dumpcap` 可选命令）、§H 范围（Linux 不在本轮）、30 分钟核心冒烟清单、缺陷记录模板；每项含前置条件、步骤、预期、自动化状态 |
| 策略报告 | `docs/perfect-writing-surface-strategy.md`（commit `e9b2256`） | §1 现状三件已成立的事、§2 差距、§3 判断（先代码质量与减法）、§4 不做清单、§5 iPhone 式原则、§6 分阶段计划、§7 三个英雄场景、§8 红队五问、§9 下一轮 spec |

两份文档均在 `e9b2256` 中入库，可随仓库直接取用。

## 8. 仍未验证（NOT RUN，不得写成 PASS）

| 项 | 状态 | 依据 / 待执行 |
|---|---|---|
| 真实中文 IME 组字与候选窗 | **NOT RUN** | `evaluation.md` L113；协议 `docs/manual-test-veil.md` B2（必须人工） |
| 100% / 125% / 200% DPI 首帧与视觉 rubric | **NOT RUN** | `evaluation.md` L20、L120；协议 E4（必须人工） |
| 实体物理打印 | **NOT RUN** | `evaluation.md` L121；协议 F3（必须人工） |
| 真实 GitHub Actions（dry-run / Windows runner / Linux runner） | **NOT RUN** | `evaluation.md` L19；`evidence/ci/actions.md`；本次 push 不等于 runner 实跑通过 |
| Linux 桌面版本 | **NOT RUN / 不在范围** | `docs/manual-test-veil.md` §H；仅 Linux CI 对 `veil-core` 做 check/test |
| 真实断网 / 零后台网络抓包 | **NOT RUN** | `evaluation.md` L23、L263；`evidence/mermaid/offline-only.md` 仅离线渲染证据；协议 G1/G2（必须人工） |

以上六项在 checklist 中保持未勾选，结论不计入 PASS；`evidence/not-run/disposition.md` 为逐项处置记录。

## 9. 引用风险

| ID | 风险 | 状态 |
|---|---|---|
| R1 | `CHANGELOG.md` L43 引用 `evidence/ux/final-v3/perf/run2.log` | **已规避**：文件保留原路径。 |
| R2 | `CHANGELOG.md` L46、`evaluation-ux.md` L7/L13 引用 `after-v2…v6`、`final…final-v4` | **部分降级**：目录与 md/json/txt 摘要仍在，实拍 PNG/PDF 已删；文档未改写。 |
| R3 | `evaluation.md` L4/L13、`CHANGELOG.md` L47/L48 原引用 `target/release/` 口径的发布记录；`README*.md` L140 现引用归档副本 | **已修复**：统一改指 `evidence/release/release-record.json`（归档副本，字节一致），并注明 `target/` 为可再生产物。 |
| R4 | `evaluation.md` L4 泛引用 `evidence/render` | 未触发（render 子目录保留，含 `dark` / `light`）。 |
| R5 | `README.md` / `README.zh-CN.md` L149 smoke 示例 `target/release/veil.exe` | 路径当前不存在（`cargo clean` 后）；命令模板语义不变，重建后即恢复。 |
| R6 | `CHANGELOG.md` L108、`docs/adr/ADR-002/011/012`、`docs/review/component-replacement-spike.md`、`docs/01-define-design.md` 引用 `spikes/...` | **已修复**：不恢复目录，全部改为「本地原型 `<name>`（已清理）」并保留结论/数字；`spikes/` 仅作为 `.gitignore` 规则名与清理记录出现。 |
| R7 | `evidence/ux/final*` 的 `corpus.pdf`（每份 26.5 MB）随历史轮次删除 | 已回收 211.9 MB；`final-v5` 的两份保留。 |

## 10. 证据保留规则（长期）

适用对象：`evidence/**`、`release-bundle/**`、根目录 `evaluation*.md`、`README*.md`、`CHANGELOG.md`。

1. **被引用即保留**：任何被 `evaluation*.md`、`README*.md`、`CHANGELOG.md`、`docs/**` 以具体路径引用的证据（摘要 md/json/csv/txt/log、二进制、截图）必须保留；若要删除，先同步改引用或降级为结论描述。
2. **中间构建证据可降级**：`_work/`、`_home/`、临时截图/日志、`*.tmp`、中间构建产物（`target/`、`target-default/`、`spikes/`）不入库；在下一轮最终证据产生后，可删除上一轮的二进制/大图，仅保留 md/json/txt/log 摘要。
3. **最终证据摘要不动**：`evidence/ux/{baseline,after,after-v7,final-v5}`、`evidence/release/`、`evidence/{render/task9-final,startup/task9-final,input,g0,ci,not-run,decision,mermaid,print}` 等被最终报告引用的摘要继续跟踪，不因清理取消跟踪。
4. **删除前必须 grep 引用**：`spikes/`、`target/release/release-record.json`、`*.png`、`*.pdf` 等删除前，先全仓库 grep 路径；有引用则先改文档再删除，确保 0 悬空引用。
5. **路径口径**：发布记录统一引用 `evidence/release/release-record.json`（归档副本）/ `evidence/release/veil.exe.sha256`；`target/release/` 下的同名文件属可再生产物，不作为文档真值。
6. **例外须记录**：`.trae` 受删除器 denylist 保护（L2），其 spec 截图清理需在删除器策略变更后另行执行，不假装已清。

## 11. 磁盘空间说明

- 工作区目录实测减少 **8,243.26 MB ≈ 8.05 GB**（16,446 → 2,656 文件）。
- 删除动作经安全包装器转入 D 盘回收站：当前 `D:\$Recycle.Bin` 约 **12.13 GB**，`D:` 可用 **458.58 GB**（2026-11-21 复核实测）。清空回收站后，对应空间才真正释放；代理无回收站清空权限。
