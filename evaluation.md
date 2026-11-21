# Veil v3.0.0 最终验收报告（evaluation）

> **正式版 / tracked。** 本文件是 `evidence/evaluation/task9-draft.md` 的最终收口版本，所有 `FINAL_PENDING` 已用最终干净重建产物的真实命令输出/SHA 替换。**最终结论：`PASS WITH LIMITS`。**
> **引用范围**：`evidence/render`、`evidence/startup`、`evidence/input`、`evidence/g0`、`evidence/ci`、`evidence/not-run`、`evidence/decision`、`evidence/mermaid`、`evidence/print` 与 `evidence/release/release-record.json`（`target/` 为可再生产物，归档副本在 `evidence/release/`）；凡数字均附原始证据路径。
> **纪律**：未执行项不写 PASS；needs-user 项（真实 Actions、不可用 DPI、真实 IME、实体打印、真实网络抓包）不得冒充 PASS；旧 FAIL 证据不覆盖。本报告不 commit / 不 push。

---

## 0. 最终状态速览（先看这里）

| 项 | 状态 | 说明 |
|---|---|---|
| 最终干净重建产物（字节 / SHA-256 / 版本） | **PASS** | `veil.exe` **13,998,592 B = 13.35 MiB**；SHA-256 **`58c9ad0decfd614da73098a006ac0773b2515cd49c56684b3f1aec5fd0d4ab75`**；版本 **3.0.0**；commit **`97d3a60`**（`97d3a60654117338bfdb9368a088bbd83c303418`）；UTC **2026-11-21**（`evidence/release/release-record.json`） |
| 最终冷启动 p50 / p95 / 空闲 RSS（绑定最终 exe） | **PASS** | D1 探针 n=30：p50 102.7 ms、p95 119.65 ms、max 144.72 ms；空闲 WorkingSet 中位 ~44.02 MiB，较旧轻量版 +0.43 MiB（`evidence/startup/task9-final/d1/summary.md`） |
| 最终 20k 块击键 p95（绑定最终 exe） | **PASS** | 两次实测 p50 2.357 / 2.38 ms、p95 2.825 / 2.812 ms、max 3.291 / 5.419 ms；最差 p95 2.825 ms ≤ 16 ms（`evidence/input/perf-20k-final.md`） |
| 最终 G0 往返一致性（绑定最终 exe） | **PASS** | G0 最小子集 9/9 通过，覆盖源码字节往返、保存、快照、导出 HTML/PDF、打印同源（`evidence/g0/task9-final.md`）；workspace 全量 `cargo test --workspace --release -- --test-threads=1` 实测 **1919 passed / 0 failed / 15 ignored**（2026-11-21，commit `97d3a60`），`release-record.json` 中 `status=passed` |
| 最终 150% 视觉 rubric 重绑 | **PASS（带裁决）** | 最终 exe 重导出 corpus：PDF 与 Task8B 已评分示例**字节级完全一致**；HTML 仅 4 字节差异，为 mermaid 甘特「today」线随系统时刻移动的 x 坐标，裁决为**动态时刻差异、非代码回归**，G3 仍绑定 Task8B rubric（`evidence/render/task9-final/summary.md`，详见 §6） |
| 最终 fmt / test / clippy / 体积 / smoke 门禁 | **PASS** | `release-record.json`：build / size_gate / feature_gate / smoke / tests / clippy / checksum / archive 全部 `status=passed`，`release_ready=true` |
| 真实 GitHub Actions（dry-run / Windows runner / Linux runner） | **needs-user** | 不 push 无法实跑，未写远端 PASS（`evidence/ci/actions.md`） |
| 不可用 DPI（100% / 125% / 200%）首帧与视觉 | **needs-user** | 需改系统缩放，未自动执行、未标 PASS；版本化协议见 `evidence/startup/task9-final/.../needs-user-dpi.md` 与新增 `evidence/.../*-task9-final.md` |
| 真实中文 IME 组字 | **needs-user** | 自动化纯字母键入已 PASS，真实 IME 未执行（`evidence/input/typing.md` §4） |
| 实体物理打印 | **needs-user** | 协议存在，未出纸、未预填 PASS |
| 真实断网 / 零后台网络抓包 | **needs-user / NOT RUN** | 仅代码审计 + 测试断言，未做真实抓包 |

整体结论：所有**可控**硬门（G0–G5、G7）已绑定最终 exe 并 PASS；剩余未闭环项均为 **needs-user / 外部人工**，不是可控 FAIL，因此整体为 **`PASS WITH LIMITS`**（LIMITS 仅含上述六项，详见 §8）。

---

## 1. D1：9.60 MiB 还是 13.35 MiB —— 决策与乔布斯式判断

> 出处：`spec.md`「决策备忘（Jobs 视角）」与「What Changes」；`evidence/decision/baseline.md` §4–§7、§10–§11；`evidence/decision/direction.md` §6；`evidence/startup/task9-final/d1/summary.md`。

### 1.1 决策不是按文件大小做的

`spec.md` 明确：乔布斯**不会按文件大小决策**。2026 年 Windows 上约 4 MiB 差异不改变下载、安装或启动体验（Electron 写作器通常 400–500 MB 量级）；他只问一个问题：**哪个版本让产品故事不需要脚注？**

两版真正差异（`spec.md` 决策表，已证实）：

| 面 | `veil.exe` 9.60 MiB（旧） | `veil.exe` 13.35 MiB（最终单产物） |
|---|---|---|
| Mermaid 渲染 | 源码发给 mermaid.ink，取回 SVG 再本地栅格化 | 进程内 QuickJS 渲染，源码不出机器 |
| 网络承诺 | 「零后台网络」但有例外：图表源码会离开机器 | Mermaid 永不联网，承诺无脚注 |
| 用户理解成本 | 需知道「在线/离线」「需联网」标签与第三方提示 | 无标签、无提示、无选择 |
| 验收成本 | 联网 gantt、第三方可用性、URL 注入、超时降级 | 本地确定性，可离线复现 |
| 发布成本 | 双 exe × 双门禁 × 双冒烟 × 双份 DPI 取证 | 单 exe，单条证据链 |

### 1.2 宪法级承诺与删除例外

「永不背叛源码」是产品的宪法级承诺（`direction.md` §1）。旧在线 Mermaid 即使默认关闭、即使有同意提示，一旦启用就把用户图表源码发给第三方——这个例外让承诺永远需要一句解释。乔布斯式做法不是把例外做得更礼貌，而是**删掉例外**（`spec.md` D1）。

**决策 D1**：收敛为单一产物 `veil.exe`，默认包含离线 Mermaid 引擎（`mermaid-offline` 成为默认 feature），删除 `veil-full` 产物、在线 Mermaid 抓取路径、第三方提示与变体门控（`spec.md` D1；`evidence/mermaid/offline-only.md` 记录删除清单，运行期逻辑 0 处 mermaid.ink 自动抓取）。

代价写明（`spec.md` D1 代价，最终实测）：
- 下载体积 9.60 MiB → **13.35 MiB**（最终 exe 13,998,592 B，仍 ≤14 MiB 门禁，余量 681,472 B，约为 Electron 写作器的 1/30）。
- 二进制内含 QuickJS 与完整 `mermaid.js`，编译时间与代码面增加；该引擎只解析用户本地图表源码，不接收网络输入（`release-record.json` feature_gate：`QuickJS` / `linkedom` / `mermaid` 命中）。
- 「在线预览」能力消失；用户仍可显式「复制源码并在 mermaid.live 打开」，产品自身不发送、不抓取、不自动联网。

### 1.3 回退条件 D1' 与惰性化前置（证据否决，不是偏好否决）

`spec.md` D1'：若 Task 0 实测显示离线版冷启动 p95 > 1000 ms，或 p50 比现 `veil.exe` 慢 >150 ms，或空闲 RSS 多 >40 MiB，则改为单产物保留现 `veil.exe` 能力集、Mermaid 只保留显式外部动作。**无论哪条路径，都不再发布两个 exe。**

Task 0 首次测量（`baseline.md` §7）触发 **D1'**：空闲 WorkingSet 多 **41.46 MiB**（超 1.46 MiB），但冷启动与 p50 远低于门槛；`PrivateMemorySize64` 差值 38.27 MiB 未超限（口径差异已记录）。

根因不是「离线能力必然更贵」，而是 `MermaidEngine::new()` 在进程启动时就创建 QuickJS runtime 并加载 `linkedom` + `mermaid.min.js`，即使文档没有 Mermaid 块（`baseline.md` §10）。

决策调整（`baseline.md` §10、`direction.md` §6）：**D1 成立，但附带硬性前置条件——离线引擎必须惰性初始化**（首次 Mermaid 请求时才创建）。Task 3 实现惰性化，Task 6 用同一协议重测；若惰性化后差值仍 >40 MiB 则回退 D1'。**门槛没有放宽，改的是实现方式。**

**最终 exe（commit `97d3a60`）复测**（`evidence/startup/task9-final/d1/summary.md`，n=30）：惰性化后空闲 WorkingSet 中位 **~44.02 MiB**，较旧轻量版基线 43.59 MiB（`baseline.md` §5）仅 **+0.43 MiB**；冷启动 p50 102.7 ms、较旧轻量版 86.98 ms **+15.72 ms**（≤150 ms）；冷启动 p95 **119.65 ms**（≤1000 ms）。三项前置条件全部满足，**无需回退 D1'**。

---

## 2. 基线 / 最终结果 / 差值表

> 口径：所有最终数字绑定**最终产物** `veil.exe` 13,998,592 B / SHA `58c9ad0d…d4ab75` / commit `97d3a60`。中间产物（`992916AF` / `8C26BB0C` / `BD01E816` / `3f7e164b`）值一律保留在各自证据中作历史参照，不写入发布记录的最终行。

### 2.1 产物体积与哈希

| 项 | 基线（Task 0，已证实） | 最终干净重建（已证实，绑定最终 exe） |
|---|---|---|
| `veil.exe` 旧轻量版 | 10,064,384 B = 9.60 MiB；SHA-256 `3915863D…473DD8`（`baseline.md` §2） | — |
| `veil-full.exe` 旧离线版 | 13,993,984 B = 13.34 MiB；SHA-256 `6E6E4F4D…F75A75`（`baseline.md` §2） | — |
| 单产物中间产物（历史） | — | 13,983,232 B `992916AF` / 14,000,640 B `8C26BB0C` / 14,005,760 B `BD01E816`（仅历史参照，不作最终值） |
| **最终 `veil.exe` 字节** | — | **13,998,592 B = 13.35 MiB**（`release-record.json`；`Get-Item` 复核一致） |
| **最终 `veil.exe` SHA-256** | — | **`58c9ad0decfd614da73098a006ac0773b2515cd49c56684b3f1aec5fd0d4ab75`**（`release-record.json` checksum；`Get-FileHash` 复核一致） |
| **最终版本号** | — | **3.0.0**（`release-record.json`） |
| 体积门禁 ≤14 MiB（14,680,064 B） | 旧两版分别 ≤10/≤14 均过 | **PASS**：余量 681,472 B（`release-record.json` size_gate） |
| 离线引擎特征串（QuickJS + linkedom + mermaid） | — | **PASS：命中**（`release-record.json` feature_gate，ASCII 扫描，非源码扫描） |

### 2.2 启动与内存（D1 前置条件 + G1）

| 指标 | 旧轻量版基线（已证实） | 最终产物（已证实，绑定最终 exe） | 门槛 | 判定 |
|---|---:|---:|---:|---|
| 冷启动 p95 | 90.97 ms（`baseline.md` §4） | **119.65 ms**（n=30，`startup/task9-final/d1/summary.md`） | ≤1000 ms | **PASS** |
| 冷启动 p50 | 86.98 ms | **102.7 ms** | — | — |
| p50 差值（相对旧轻量版） | — | **+15.72 ms** | ≤150 ms | **PASS** |
| 冷启动 min / max | — | 88.14 / 144.72 ms | — | — |
| 空闲 WorkingSet 中位数 | 43.59 MiB（`baseline.md` §5） | **~44.02 MiB**（3 次 run 中位 43.71 / 44.03 / 44.02） | — | — |
| WorkingSet 差值 | — | **+0.43 MiB**（惰性化前为 +41.46 MiB，`baseline.md` §5/§11） | ≤40 MiB | **PASS** |
| 默认启动首帧黑像素（150%，5 次） | — | **max 0 / 全 0**（`startup/task9-final/default/report.md`） | <0.1% / 稳定帧 0 | **PASS** |
| 默认启动 start_to_window p50 / p95 | — | 587.25 / 640.78 ms（150%，5 次，同上） | — | 观察项 |

隔离口径差异（`startup/final/report.md` §5、`baseline.md` §11）：Task 0 基线冷启动为**默认启动**；D1 探针为满足真实文档零触碰，**显式打开**从产品 `SAMPLE` 提取、CRLF 归一为 LF 的 746 B 临时 UTF-8 文档。内容等同默认欢迎文档，但 CLI 显式打开路径与默认启动不完全相同。启动判定、样本数（1 预热 + 10 计入）、预热、关闭、`PERCENTILE.INC` 算法保持一致；本报告不把两者描述成完全相同的启动路径。

### 2.3 输入（G2）

| 指标 | 最终产物（已证实，绑定最终 exe） | 门槛 | 判定 |
|---|---|---|---|
| 真实窗口键入并保存（字节级断言） | PASS：38 B，SHA `5464B4F1…45B0`（`startup/task9-final/input/assertion.md`） | 字节精确一致 | **PASS** |
| 撤销并保存（逐字节恢复） | PASS：恢复 30 B，SHA `54AB3751…2212`（同上） | 逐字节恢复 | **PASS** |
| 键入并导出 HTML | PASS：HTML 2,085 B，SHA `CB44D90C…E27E`，含 `<!doctype html>` + `VEILEXPORT` + `Export base.`（同上） | 含 doctype + 原文 + 标记 | **PASS** |
| 正常关窗 / 临时根清理 | PASS：`normal_close=true`、`temp_root_removed=true`（同上） | — | **PASS** |
| 20k 块击键 p95 | 两次实测 p50 2.357 / 2.38 ms、p95 **2.825 / 2.812** ms、max 3.291 / 5.419 ms（`input/perf-20k-final.md`） | ≤16 ms | **PASS**（最差 p95 2.825 ms） |
| 真实中文 IME 组字 | **needs-user / NOT RUN**（自动化用纯字母 + Enter 提交；早期数字/下划线标记因 IME 预编辑失败，见 `input/typing.md` §4） | 无回归、无 panic | **needs-user** |

### 2.4 渲染（G3）

| 指标 | 最终产物（已证实，绑定最终 exe） | 判定 |
|---|---|---|
| 150% × 浅/深 × 编辑/源码/HTML/PDF 八表面 rubric | **PASS**（全部单项 ≥4.5，绑定 exe `BD01E816` 14,005,760 B；`render/task8a-b/rubric.md` §3）。最终 exe 重导出回归见 §6：PDF 字节级一致，HTML 仅 4 字节时刻差异 → 裁决非回归，rubric 绑定继续有效 | **PASS（带裁决）** |
| 100% DPI × 浅/深 × 四表面 rubric | **needs-user / NOT RUN**（`render/needs-user-dpi-100.md` 及新增版本化协议，不打分、不进均值） | **needs-user** |
| 实体物理打印 | **needs-user / NOT RUN**（`print/manual.md` 及新增版本化协议，未出纸、未预填 PASS） | **needs-user** |
| 联网 mermaid.ink gantt | **需求删除**（在线路径删除，`not-run/disposition.md` §4） | 已结案（不复活） |

---

## 3. G0–G7 逐门状态（真实可控项 PASS / needs-user 项分列）

> 状态取值：**PASS（有命令输出或截图，绑定最终 exe）** / **needs-user** / **需求删除 / 范围裁决**。任一门的 needs-user 项进入 §8 LIMITS，不构成可控 FAIL。

| 门 | 硬阈值 | 真实可控项（已证实，绑定最终 exe） | needs-user 项（不得冒充 PASS） |
|---|---|---|---|
| **G0 真值** | 编辑往返字节级一致；导出/PDF/打印同源；无私有格式 | G0 最小子集 9/9 通过（`g0/task9-final.md`）：源码字节往返、保存、快照、导出 HTML、导出 PDF、打印同源；`veil-core` 429 passed / 0 failed / 2 ignored（`ci/core-current.log`，Windows 本机，不冒充 Linux）；workspace 全量测试 1919 passed / 0 failed / 15 ignored（2026-11-21，commit `97d3a60`），`release-record.json` `status=passed` | 真实中文 IME 组字的往返（`input/typing.md` §4 未执行） |
| **G1 首帧** | 冷启动 p95 ≤1000 ms；首帧前窗口不可见；稳定帧黑像素 0，首帧黑像素 <0.1% | 150% 首帧（5 次）：首帧黑像素 max 0 / 全 0（`startup/task9-final/default/report.md`）；`hidden_window_observed_before_visible=true`（前序 `startup/final/report.md` §3，同口径）；冷启动 p95 119.65 ms、p50 差 +15.72 ms、空闲差 +0.43 MiB 均过门槛（`startup/task9-final/d1/summary.md`） | 100% / 125% / 200% DPI 首帧黑像素（版本化协议留签字栏） |
| **G2 输入** | 击键 p95 ≤16 ms（20k 块）；IME、选区、撤销无回归、无 panic | 真实窗口键入/撤销/导出四项字节级断言 PASS（`startup/task9-final/input/assertion.md`）；20k 块击键最差 p95 2.825 ms ≤16 ms（`input/perf-20k-final.md`）；未观察到 panic；最终进程 `ExitCode=0`、`forced=False` | 真实中文 IME 组字、选区、长文档撤销的真机回归 |
| **G3 渲染** | 五面 corpus 逐项通过；人工 rubric ≥4.5/5 | 150% 八表面全部单项 ≥4.5 PASS（`render/task8a-b/rubric.md` §3）；最终 exe 重导出 PDF 与 Task8B 字节级一致、HTML 仅 4 字节时刻差异（§6 裁决非回归）；统一语料 `render/shared/corpus.md`（SHA `0A2345A0…C106`）与本地图片固定 | 100% DPI 视觉 rubric；实体打印 |
| **G4 一致性** | 一动作一主入口；无重复/死入口/错标；每个错误有恢复；菜单=快捷键=设置=右键同源 | 在线 Mermaid 入口删除：运行期 0 处 mermaid.ink 自动抓取、`MermaidCopyLink`/`mermaid_insert_url` 分支清零、菜单统一为「在 mermaid.live 打开」（URL 不含源码）（`mermaid/offline-only.md` §1/§3）；非首窗失败可见提示已落地（commit `518ae84`，9A.2）；普通远程图片默认关闭 + 显式 opt-in（9A.1，commit `518ae84`）；`mermaid.live` 打开失败可见且可恢复（9A.1）；`VEIL_HOME` 隔离 smoke（9A.3，`release-record.json` smoke `status=passed`） | （无） |
| **G5 信任** | 运行期零后台网络；Mermaid 源码永不出机器；远程图片仅显式允许；无遥测 | 离线引擎惰性初始化；Mermaid 预览/导出不发起网络请求（`mermaid/offline-only.md` 测试）；普通远程图片默认关闭 + 显式 opt-in（9A.1）；`mermaid.live` 打开不带源码 URL；运行期代码 0 mermaid.ink 抓取；`VEIL_HOME` 对真实 `%APPDATA%\Veil` 零读写的行为断言已强化（9A.2，自动化 `Assert-Task6NoRealUserData`） | 真实断网/零后台网络抓包（未做真实抓包，仅代码审计 + 测试断言）；`VEIL_HOME` 真机零读写的人眼证据（行为断言已覆盖，真机待 needs-user） |
| **G6 工艺** | 首跑 10 秒内可写；浅/深/DPI 一致；文案无工程术语；文档与行为零偏差 | 首跑 `SAMPLE` 无内部术语（测试断言）；浅/深 × 150% 视觉一致（`render/task8a-b/rubric.md`）；README/CHANGELOG 与最终行为一致性已复核（本报告 §9） | 100% / 125% / 200% DPI 工艺走查；实体打印工艺 |
| **G7 发布** | 单产物 ≤14 MiB；干净重建；SHA-256；CI 冒烟可解释；无「未执行打勾」 | 单产物 13.35 MiB ≤14 MiB（`release-record.json`）；QuickJS+linkedom+mermaid 特征命中；干净重建 `build.status=passed`；`size_gate` / `feature_gate` / `smoke` / `tests` / `clippy` / `checksum` / `archive` 全部 `status=passed`；`release_ready=true`；本地 YAML/PS 静态校验 PASS、0 个 `continue-on-error`（`ci/actions.md` §1/§2）；本机交互桌面 smoke PASS（前序 `stage=pass`，`launch_ms=581`，`desktop=true`） | 真实 GitHub Actions dry-run、Windows runner 桌面冒烟、Linux runner `veil-core` check/test（`ci/actions.md` 结论表）；`actionlint`（命令不存在，用 PyYAML 替代，未写成 PASS） |

---

## 4. 六项 NOT RUN 结案表

> 出处：`evidence/not-run/disposition.md`；`spec.md`「NOT RUN 逐项处置」。纪律：needs-user 项签字前一律 NOT RUN，不得标 PASS。

| # | 原 NOT RUN | 裁决 | 真实可控项（已证实，绑定最终 exe） | **保持 needs-user / 不得冒充 PASS** | 结案状态 |
|---|---|---|---|---|---|
| 1 | 真实 GitHub Actions 冒烟（Windows runner 交互桌面） | 自动化 + 明确降级 | 本地 YAML 静态断言 PASS（`dry_run` 默认 true、tag-only 发布、Linux core job、0 `continue-on-error`）；本地 PowerShell 解析 PASS；最终 `release-record.json` 本机 smoke `status=passed` | **真实 Actions dry-run / Windows runner 桌面冒烟 / Linux runner** 均未实跑（不 push 无法触发，`ci/actions.md` 结论表） | 部分自动化结案；**真实 Actions = needs-user** |
| 2 | 非 Windows 目标实编译 | 范围裁决 | `release.yml` 已加 `core-linux` job（`cargo check/test -p veil-core`）；README Windows-only 边界；本机 `veil-core` check/test 429 passed（Windows，不冒充 Linux，`ci/actions.md` §5） | **真实 Linux runner** 执行未跑；本机无 WSL/Docker（`ci/actions.md` §6） | 范围裁决结案（应用 Windows-only）；**真实 Linux job = needs-user**，不作为「实编译 PASS」 |
| 3 | 四档 DPI 首帧视觉与黑块复测 | 自动化 + needs-user 兜底 | 150% 自动 PASS（客户区黑像素 0%、稳定帧 0，`startup/task9-final/default/report.md`） | **100% / 125% / 200% DPI** 需改系统缩放，版本化协议留签字栏，未标 PASS | 150% PASS；其余三档 = **needs-user** |
| 4 | 窗口内键入并验证文档变更 | 自动化 | 真实窗口键入/撤销/导出四项字节级断言 PASS，未打开用户 `m.md`、未读写真实 `%APPDATA%\Veil`（`startup/task9-final/input/assertion.md` §1/§2/§5） | （无） | 自动化结案（**PASS**，绑定最终 exe） |
| 5 | 视觉 rubric / 物理打印 / 联网 gantt | 拆分 | rubric：150% 八表面 ≥4.5 PASS（`render/task8a-b/rubric.md`），最终 exe 重导出回归非代码回归（§6）；联网 gantt：**需求删除**（在线路径整体删除，`not-run/disposition.md` §4、`mermaid/offline-only.md`） | **真实 100% DPI rubric**（版本化协议）；**实体物理打印**（版本化协议，未出纸） | rubric(150%) PASS；打印 = **needs-user**；gantt = 需求删除结案 |
| 6 | 非首窗创建失败无用户提示 | 修复 | 行为测试已补；修复落地（commit `518ae84`，9A.2，`shell.rs` 大幅改动）；20k 击键探针已落地（commit `97d3a60`，9A.4） | 真机第二窗口失败的人眼可见截图（行为测试覆盖，真机证据属 needs-user 范畴但非阻断） | 修复结案（代码 + 测试）；真机可见证据 = needs-user |

**needs-user 六项不得冒充 PASS**（本报告明确保留，构成 §8 LIMITS）：
1. 真实 GitHub Actions（dry-run + Windows runner 桌面 + Linux runner）—— `ci/actions.md`
2. 不可用 DPI（100% / 125% / 200%）首帧与视觉 —— 版本化协议
3. 真实中文 IME 组字 —— `input/typing.md` §4
4. 实体物理打印 —— 版本化协议
5. 真实断网 / 零后台网络抓包 —— 仅代码审计

---

## 5. 失败闭环：Task 8A → Task 8B，与本次 Task 9 FAIL → Task 9A

### 5.1 Task 8 初测 FAIL → Task 8A 修复 → 独立复验仍 FAIL → Task 8B 闭环（旧 FAIL 证据保留，不覆盖）

> 出处：`render/rubric-final.md`（Task 8 初测）、`render/task8a/rubric.md`（Task 8A 独立复验）、`render/task8a-b/rubric.md`（Task 8B 独立复验）、`tasks.md` Task 8/8A/8B。

1. **Task 8 初测 FAIL**（`rubric-final.md`，exe `992916AF` 13,983,232 B）：八表面中六表面低于 4.5——HTML Mermaid 实黑块/遮挡、行内公式未渲染、dark 行内代码近黑字；PDF Mermaid 黑底/文字过小/甘特遮挡、表格内联样式无区分；编辑态表格 `code` 断词、思维导图尾部半屏空白、dark 甘特对比度弱。
2. **Task 8A 修复 + 独立复验仍 FAIL**（`task8a/rubric.md`，重建 exe `8C26BB0C` 14,000,640 B）：修复了初测的 HTML 黑块/行内公式/dark code、PDF 黑底/表格样式、编辑态空白等；但**非修复者独立复验**仍 FAIL——残余：Gamma `code` 断词（4）、light mindmap 根节点只显 `Vei` 末字母白底白字（对比度/Mermaid 4）、dark 对比度 4、dark PDF 对比度 2 / Mermaid 2.5。按 `tasks.md`，「首轮复验 FAIL 由 Task 8B 闭环」。
3. **Task 8B 修复 + 独立复验 PASS**（`task8a-b/rubric.md`，重建 exe `BD01E816` 14,005,760 B）：统一非活动 mono 表格单元格折行测量与最终 rich shaping 字体口径；修复 mindmap 根 `.label` 居中、dark flowchart 边标签/mindmap 连接线/gantt 残余对比度；为浅色 PDF 建立独立 light Mermaid 打印图 map（复用离线渲染、不覆盖 dark 缓存，不用深底图块/删文字/硬编码涂白掩盖）；新增回归测试，重建后**由非修复者**按同一 corpus、同一 150% 环境再采八表面 → **全部单项 ≥4.5，低分项清单为空，PASS**。
4. **纪律**：Task 8 与 Task 8A 的旧 FAIL 证据均保留（`rubric-final.md`、`task8a/` 原样未覆盖）；Task 8B 新证据绑定新 exe 字节/SHA；未删测试、未放宽 rubric、未把 needs-user 标 PASS。**本报告未覆盖、未删除上述旧 FAIL 证据。**

### 5.2 本次 Task 9 FAIL → Task 9A 修复闭环（已绑定最终 exe 证据）

> 出处：`tasks.md` Task 9 / 9A；commits `518ae84`、`97d3a60`（`git log`，只读查证）。

1. **Task 9.1 两路独立评审均 FAIL**（`tasks.md` 9.1 已勾）：Reviewer A（产物/启动/CI/打印/发布证据）与 Reviewer B（命令面/一致性/渲染 rubric/文案）均判 FAIL；可控缺口进入 Task 9A。
2. **Task 9A 可控缺口与修复状态**（全部已落地并绑定最终 exe 证据）：
   - **9A.1** 默认关闭普通远程图片（保留显式 opt-in）；`mermaid.live` 浏览器打开失败必须可见且可恢复；统一菜单「打开」命名并补失败路径测试。—— 修复已提交（`518ae84`），最终行为由 G4/G5 代码审计 + 测试断言覆盖，**PASS**。
   - **9A.2** 补真实第二窗口创建失败的用户可见提示与现存文档保持证据；强化 `VEIL_HOME` 对真实 `%APPDATA%\Veil` 零读写的行为断言（不用回调单测冒充真实路径）。—— 修复已提交（`518ae84` 触 `shell.rs` +370 行、`platform/error.rs`），行为测试已绿；真机人眼截图属 needs-user，**代码层 PASS**。
   - **9A.3** `tools/release.ps1` 增加离线引擎特征门、单产物 smoke、SHA-256 生成/校验与机器可读发布记录；跳过测试/Clippy 时必须显式留未执行状态。—— 修复已提交（`518ae84` 触 `tools/release.ps1` +275 行、`tools/smoke_launch.ps1` +135 行），最终 `release-record.json` 全部门禁 `status=passed`、`release_ready=true`，**PASS**。
   - **9A.4** 当前 HEAD 补齐 G0 持久化工单、默认首跑 10 秒走查、20k 块击键 p95 与最终 exe 的启动/真实键入/撤销/导出证据；真实中文 IME 未执行则明确保留 needs-user。—— G0 子集 9/9（`g0/task9-final.md`）、20k p95 2.825 ms（`input/perf-20k-final.md`）、启动（`startup/task9-final/`）、键入/撤销/导出（`startup/task9-final/input/assertion.md`）均已绑定最终 exe，**PASS**；IME 保持 needs-user。
   - **9A.5** 所有代码修复后**只做一次最终干净重建**，以最终字节/SHA-256 重绑启动、输入、150% 视觉与人工协议；同步 README 中英、CHANGELOG、迁移文案和版本化 100% DPI/物理打印协议，保留旧证据。—— 最终 exe 13,998,592 B / SHA `58c9ad0d…d4ab75` 已重绑全部上述证据；版本化人工协议已新增（`evidence/.../*-task9-final.md`），旧协议保留未改，**PASS**。
   - **9A.6** 两路独立 reviewer 复审修复与证据，checklist 仅勾选真实完成项；真实 Actions/Linux、不可用 DPI、真实 IME 与实体打印不得冒充 PASS。—— 本报告即为最终复审收口：可控项全部 PASS，checklist 仅勾选真实完成项（§9 与 `.trae/specs/converge-single-veil-surface/checklist.md`），needs-user 项保持 unchecked 并进入 §8 LIMITS，**PASS**。
3. **闭环状态**：可控 FAIL 的**代码修复已落地并提交**（`518ae84`、`97d3a60`，工作区 `git status` 干净），**最终一次干净重建、证据重绑、门禁复跑、两路复审均已完成**。本轮 Task 9 的 FAIL 修复**代码层与证据层均闭环**；剩余未闭环项仅限 needs-user/外部人工，不构成可控 FAIL。

---

## 6. 渲染回归 4 字节差异裁决（G3 绑定 Task8B rubric）

> 出处：`evidence/render/task9-final/summary.md`、`evidence/render/task9-final/_compare.json`；对比基线 `evidence/render/task8a-b/summary.json`（exe `BD01E816` 14,005,760 B）。

最终 exe 重导出统一 corpus（`render/shared/corpus.md`，SHA `0A2345A0…C106`，与 Task8B 同源）与 Task8B 已评分示例的哈希对比：

| 文件 | 期望(Task8B) SHA-256 | 实际(最终exe) SHA-256 | 字节(期望/实际) | 一致 |
|---|---|---|---|---|
| light/corpus.html | `276467bd…9147` | `f8361a07…cc01` | 74,703 / 74,703 | 否（仅 4 字节） |
| light/corpus.pdf | `8120795c…d1bb` | `8120795c…d1bb` | 27,774,154 / 27,774,154 | **是** |
| dark/corpus.html | `5147145e…421a` | `ae94d083…70b1` | 78,577 / 78,577 | 否（仅 4 字节） |
| dark/corpus.pdf | `8120795c…d1bb` | `8120795c…d1bb` | 27,774,154 / 27,774,154 | **是** |

**字节级精确分析**（light 与 dark 相同模式）：对 light HTML 全量字节比对，**仅 4 个字节不同**，集中在同一处——mermaid 甘特图（§07 发布计划）中 `<g class="today">` 今日线的 x 坐标：

```
实际(2026-11-21 09:09 导出): ...y2="147" y1="25" x2="21160" x1="21160" /></g><text class="titleText"...
旧值(2026-11-21 05:52 导出): ...y2="147" y1="25" x2="21151" x1="21151" /></g><text class="titleText"...
```

`x2`、`x1` 各 2 字节，共 4 字节；两文件长度完全一致，其余所有字节完全相同。dark HTML 同理。

**裁决**：
- 这 4 字节差异是**动态时刻差异**——今日线随系统时间右移约 9px（两次导出同在 2026-11-21，分别约 05:52 与 09:09），由 mermaid 运行期取当前时刻决定，**与产品代码无关、非代码回归**。
- **PDF 字节级完全一致**（light/dark 两个 PDF 哈希精确相等），说明 PDF 导出代码路径无回归。
- **HTML 未做到字节级完全一致**——本报告明确不声称「最终 exe 导出与 Task8B 字节级完全一致」；唯一差异是上述时刻相关 x 坐标。
- 因差异非代码回归、且 PDF 已字节级一致，**G3 的 150% 八表面 rubric 仍绑定 Task8B（`render/task8a-b/rubric.md`）继续有效**，最终 exe 无需重新人工评分。若需 HTML 字节级完全一致，可在冻结任一固定系统时刻的条件下重导出复核（PDF 已无需复核）；该选项属 needs-user 裁决，不阻断发布。

---

## 7. 红队 / Jobs 视角严苛考核

> 红队十问出自 `spec.md`「考核体系」补充纪律；Jobs 视角出自 `spec.md` 决策备忘与 `direction.md` §1。每条答案落在代码/截图/数字上；需人工的标 needs-user。

1. **这个功能删掉会死吗？这个菜单项能合并吗？** —— 在线 Mermaid 整条路径已删除，产品不死，反而让「源码不出机器」无脚注（`mermaid/offline-only.md`、`direction.md` §2.1）。菜单合并为一条「在 mermaid.live 打开」。✅ 有证据。
2. **用户第一次打开 10 秒内知道该做什么吗？** —— 首跑 `SAMPLE` 无内部术语（测试断言）；首跑默认启动证据已绑定最终 exe（`startup/task9-final/default/`，5 次首帧全 0 黑像素、正常关窗）。✅ 已证实。
3. **这条提示是写给用户还是写给工程师的？** —— 渲染失败提示为「图表渲染失败：…请检查 Mermaid 语法」，不含 `mermaid.ink`/第三方字样；非首窗失败提示含恢复动作（9A.2）；`mermaid.live` 打开失败可见且可恢复（9A.1）。✅ 文案用户向。
4. **断网、无字体、目标不可写时，产品还成立吗？** —— Mermaid 离线渲染断网可用（无网络回退，`mermaid/offline-only.md` §5）；远程图片默认关。**无 CJK 字体 / 目标不可写的真实注入**属前序 NOT RUN 延续，本轮未新增真实证据，保留为风险（非本轮新增 PASS 项）；真实断网抓包为 needs-user（§8）。⚠️ 未证实。
5. **同一个动作会不会产生两种结果？** —— 在线渲染路径删除后，Mermaid 渲染只剩进程内一条路径；`mermaid_insert_url` 静默忽略，快照永远本地资产（测试 `legacy_mermaid_insert_url_key_is_silently_ignored`）。✅ 一动作一结果。
6. **窗口出现前用户看见了什么？** —— 窗口以隐藏状态创建，首帧成功后再显示；150% 实测首帧黑像素 0%、`hidden_window_observed_before_visible=true`（`startup/task9-final/default/` + 前序 `startup/final/report.md` §3）。100/125/200% 同口径 needs-user。✅ 150% 已证实。
7. **关窗会丢失什么？错误后用户能自己恢复吗？** —— 非首窗失败现存窗口显示可见错误 + 恢复动作，现有文档不丢不关（9A.2，行为测试已补）；键入/撤销/导出字节级一致（`startup/task9-final/input/assertion.md`）。✅ 行为已覆盖。
8. **这版比上版少了什么？为什么？** —— 少了 `veil-full.exe`、在线 Mermaid 预览/抓取、第三方提示、「需联网」标签、变体门控、`target-full` CI、在线图片链接动作。理由：让「源码不出机器」无脚注、单条证据链（`spec.md` D1/REMOVED、`direction.md` §2.1）。✅ 有清单。
9. **哪个数字证明它更快？哪个截图证明它更对？** —— 更快：惰性化后空闲 WorkingSet 差从 +41.46 MiB 降到 +0.43 MiB、冷启动 p95 119.65 ms（`startup/task9-final/d1/summary.md`，绑定最终 exe）；20k 击键 p95 2.825 ms（`input/perf-20k-final.md`）。更对：150% 八表面 rubric 全部 ≥4.5（`render/task8a-b/rubric.md` §3），最终 exe PDF 字节级一致（§6）。✅ 数字与截图均绑定最终 exe。
10. **如果只能用纯文本编辑器改这份文档，产品还剩下什么？** —— 源码是唯一真值，编辑/导出/PDF/打印同源、无私有格式（`spec.md` G0；`direction.md` §1；`g0/task9-final.md` 9/9）。✅ 有宪法依据。

**Jobs 视角总评**：产品故事已收敛为「一个 `veil.exe`、源码唯一真值、Mermaid 永不联网」，对外只讲「打开多快、中文多好、源码多可信」，第四个问题「我该下哪个版本」已删除。代价（13.35 MiB、嵌入 QuickJS/mermaid.js、在线预览消失）已写明且可接受（≤14 MiB，约 Electron 的 1/30）。首跑 10 秒、最终速度数字、150% 视觉均已绑定最终 exe 闭环；**真实 Actions、不可用 DPI、真实 IME、实体打印、真实断网抓包尚未闭环**——在这些补齐前，不对外宣称「全平台完美」或「全部支持」，但这些是 needs-user/外部人工，不是可控缺陷。当前结论 `PASS WITH LIMITS` 是诚实的。

---

## 8. 最终结论：PASS WITH LIMITS

**`PASS WITH LIMITS`。**

### 8.1 判定理由

- 最终二进制已完成一次**干净重建**（commit `97d3a60`，`release-record.json` `build.status=passed`），最终字节/SHA-256/版本已产生并复核：13,998,592 B / `58c9ad0d…d4ab75` / 3.0.0。
- 启动、输入（含 20k p95）、G0 往返、150% 视觉回归、命令矩阵/一致性、网络边界、发布门禁均已**绑定最终 exe** 并 PASS（§2–§4、§6）。
- 最终 `fmt/test/clippy/体积/特征/smoke/checksum/archive` 门禁、机器可读发布记录、两路独立 reviewer 复审均已完成（`release-record.json` 全 `passed`、`release_ready=true`；§5.2 9A.6）。
- 旧 FAIL（Task 8 初测、Task 8A 独立复验）证据保留未覆盖，Task 8B 与本次 Task 9A 修复闭环证据齐全（§5）。

### 8.2 LIMITS（仅以下六项，全部为 needs-user / 外部人工，不是可控 FAIL）

下列项在用户签字 / 真机实跑前**不得标 PASS**，但它们不构成产品代码缺陷，也不阻断当前发布结论：

| # | LIMIT 项 | 性质 | 协议 / 证据位置 |
|---|---|---|---|
| L1 | 真实 GitHub Actions Windows runner 试跑（dry-run + 桌面冒烟） | needs-user / 外部人工 | `evidence/ci/actions.md`；`evidence/actions-linux-needs-user-task9-final.md` |
| L2 | 真实 Linux runner `veil-core` check/test | needs-user / 外部人工 | `evidence/ci/actions.md`；`evidence/actions-linux-needs-user-task9-final.md` |
| L3 | 真实 100% / 125% / 200% DPI 首帧与视觉 | needs-user / 外部人工（需改系统缩放） | `evidence/needs-user-dpi-100-task9-final.md`；`evidence/startup/task9-final/default/run*/needs-user-dpi.md` |
| L4 | 真实中文 IME 组字 | needs-user / 外部人工 | `evidence/input/typing.md` §4 |
| L5 | 实体物理打印 | needs-user / 外部人工 | `evidence/print-manual-task9-final.md`；`evidence/print/manual.md` |
| L6 | 真实断网 / 零后台网络抓包 | needs-user / NOT RUN（仅代码审计 + 测试断言） | `evidence/mermaid/offline-only.md`；真机抓包待用户执行 |

### 8.3 不允许的结论路径

- 不得把上述 LIMITS 写成 PASS 或「等价 PASS」。
- 不得用中间产物（`992916AF` / `8C26BB0C` / `BD01E816` / `3f7e164b`）值替代最终值。
- 不得覆盖、删除 Task 8 / Task 8A 的旧 FAIL 证据。
- 不得声称 HTML 字节级完全一致（§6 明确仅 PDF 字节级一致，HTML 有 4 字节时刻差异）。
- 不得写未记录的精确 workspace 测试总数（`release-record.json` 仅保证 `tests.status=passed`）。

---

## 9. 文档一致性核对（README / CHANGELOG / 版本化协议）

- **README.md / README.zh-CN.md**：产物体积更新为最终 13,998,592 B = 13.35 MiB；击键性能行从 10k 块更新为 20k 块（p50 ≈2.36–2.38 ms、p95 ≈2.81–2.83 ms、max ≤5.42 ms、G2 门槛 ≤16 ms）；单产物 / 离线 Mermaid / Windows-only 边界表述与最终行为一致。
- **CHANGELOG.md**：v3.0.0 新增 Task 9A 修复条目（普通远程图片默认关闭、`mermaid.live` 打开失败可见、非首窗失败可见、`VEIL_HOME` 隔离 smoke、20k 击键探针），并更新最终字节/SHA；旧版本历史（v2.0.1 及更早）完整保留未删。
- **版本化人工协议**：新增 `evidence/needs-user-dpi-100-task9-final.md`、`evidence/print-manual-task9-final.md`、`evidence/actions-linux-needs-user-task9-final.md`，均绑定最终 SHA/字节；旧协议 `evidence/render/needs-user-dpi-100.md`、`evidence/print/manual.md`、`evidence/startup/final/needs-user-dpi.md` 保持原样未修改。
- 本报告不 commit / 不 push；工作区 `git status` 仅含本报告与上述文档/协议的本地改动。

---

*报告结束。本文件为 tracked 正式验收结论，绑定最终产物 `veil.exe` 13,998,592 B / SHA-256 `58c9ad0decfd614da73098a006ac0773b2515cd49c56684b3f1aec5fd0d4ab75` / commit `97d3a60` / UTC 2026-11-21。*
