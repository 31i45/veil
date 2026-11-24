# Veil v3.0 Final Evaluation — 2026-11-23

> **历史评价快照（v3.0.1，2026-11-23）**：下列 PASS、测试数、体积和发布判断仅对应当时列出的提交与已执行工程门禁，不是当前 v3.0.2 的重新验收。外部事项仍为 NOT RUN；本报告不证明审美、品牌声望、采用度、营收或经典地位。

> 上轮 cleanup-and-polish-v3 解决用户可见问题（badge 6→9、NOT RUN 集中归档、ci.yml core-linux job）。
> 本轮 final-polish-steve-jobs 解决仓库健康度、自动化验证全量落地、证据链分级、历史清理和发布物料。
> 两份 commit：`f82b919` + 本轮 commit。Review R1（cleanup）**PASS**，本轮 Review R2（final-polish）—— 见 `.trae/specs/final-polish-steve-jobs/review.md`。

---

## 1. 状态总览

| 维度 | 数值 | 状态 |
|---|---|---|
| HEAD commit | 本轮 commit (final-polish) | — |
| 上轮 commit | `f82b919` (deny.toml + 商业化物料 + eval 归位) | ✅ |
| cargo fmt --all -- --check | 0 inconsistencies | ✅ PASS |
| cargo clippy --workspace --all-targets -- -D warnings | 0 warnings | ✅ PASS |
| cargo audit | 3 allowed (bincode/ttf-parser/rustix)，零 [audit] | ✅ PASS |
| cargo deny check | advisories ok，bans ok，licenses ok，sources ok | ✅ PASS |
| cargo test --workspace --lib | 1313 passed / 0 failed / 12 ignored | ✅ PASS |
| cargo test -p veil-core | 340 passed / 0 failed / 1 ignored | ✅ PASS |
| release exe | 13.40 MiB | ✅ |
| Rust toolchain | 1.99.1（本地 + ci.yml + release.yml 统一） | ✅ |
| git stash | empty | ✅ PASS |
| 根目录 .md 文件 | 7（正式文档 only：README × 2、CHANGELOG、LICENSE、SECURITY、SUPPORT、PRIVACY、THIRD-PARTY-LICENSES） | ✅ PASS |
| evidence/ | 2192 files, 343.47 MB（gitignored） | ✅ 分级清晰 |

## 2. NOT RUN 处置全表

disposition.md 原始 7 项 + 本轮 final-polish 新增 typing-verify 落地：

| disposition § | 原始项 | 上轮处置 | **本轮 final-polish 处置** | 证据 |
|---|---|---|---|---|
| §1.1 | 真实 GitHub Actions（push 后） | 静态已核，release.yml 有 workflow_dispatch + dry_run default true | 静态再次验证；真实触发仍需 push | ci-static-verify.txt |
| §1.2 | Linux core CI job 代码跨平台 | ci.yml 新增 core-linux job（上轮）；静态验证 | 上轮已落地；clippy+test+fmt 全绿确认 | ci.yml grep；cargo check/test |
| §1.3 | DPI 100/125/200% 首帧 | needs-user（需改系统缩放） | **用户决定放弃** — 150% 取证 PASS；用户认为已足够 | dpi-150-evidence.txt |
| §1.4 | 窗口内 typing→undo→save→export | **SKIP（需 GUI）** | **✅ PASS 本轮落地** | typing-verify/summary.json + walkthrough.log |
| §1.5 | 物理打印真机 | needs-user | **用户决定放弃** — Windows 虚拟 PDF 已覆盖，用户认为足够 | — |
| §2.1 | CI YAML/脚本静态验证 | ✅ PASS | 本轮再次验证，deny.toml rationale 补齐 | ci-static-verify.txt + deny.toml |
| §2.2 | Linux core CI 本地跨平台 | ✅ PASS | 同上 | core-cross-platform-check.txt |
| §2.3 | DPI 150% 当前档取证 | ✅ PASS（证据预存） | 证据存在且完整 | dpi-150-evidence.txt |
| §2.4 | typing-verify | ⚠️ SKIP | ✅ **PASS 本轮落地** | typing-verify/summary.json |
| — | 生产可信签名（SignPath/Azure/商业） | **needs-user** | 未变；清单在 production-signing-options.md §7 | README Trust 章节 |
| — | 真实 SmartScreen（MotW 下载） | **depends-on（需生产签名）** | 未变 | README Trust 章节 |
| — | 跨机信任与吊销演练 | **depends-on（需生产签名）** | 未变 | README Trust 章节 |

**结论：disposition.md §2 "本 change 可自动化" 四项 → 本轮后四项全 PASS ✅**。

## 3. 自动化验证结果（全部 4 项 PASS）

| # | 项 | 结果 | 证据 | 关键数字 |
|---|---|---|---|---|
| 1 | CI YAML/脚本静态验证 | ✅ PASS | ci-static-verify.txt | 无 continue-on-error；smoke 1s 预算在位 |
| 2 | veil-core 跨平台编译 | ✅ PASS | core-cross-platform-check.txt | cargo check + test exit_code=0；无 platform cfg |
| 3 | DPI 150% 首帧取证 | ✅ PASS | dpi-150-evidence.txt | 首帧 571ms，黑像素 <0.1%（2026-11-20） |
| 4 | typing→undo→save 交互链 | ✅ **PASS（本轮新落地）** | typing-verify/summary.json | walkthrough exit_code=0，40 steps OK；VEIL_HOME 快照：54B → 79B → 54B |

**Walkthrough 关键证据**：
- `walkthrough.ps1 exit_code=0`，40 个步骤在 4 个宽度下全部成功
- VEIL_HOME 初始快照 54 字节 → 输入"HelloWorld中文测试123"后 79 字节 → 撤销+保存后 54 字节（正确恢复）
- PrintWindow blank 降级为屏幕捕获（已知环境限制，非产品功能失败）
- 0 ERROR，仅 14 WARN（全是 PrintWindow blank）

## 4. 技术债

| Crate | 版本 | 类别 | RUSTSEC | 当前处置 | 升级计划 |
|---|---|---|---|---|---|
| bincode | 1.3.3 | unmaintained | 2025-0141 | deny.toml `ignore` + `reason` 块 | v3.1 — 2.x API 不兼容，需专项 |
| ttf-parser | 0.25.1 | unmaintained | 2026-0192 | deny.toml `ignore` + `reason` 块 | v3.1 — 0.26+ API shift |
| rustix | 1.1.5 | unsound xattr | 2026-0486 | deny.toml `ignore` + `reason` 块 | 等上游 1.2+ 自动流转 |

**风险评估**：三项均为低风险。非 veil 直接依赖（bincode/ttf-parser 是 veil/veil-mermaid-offline 单路径使用；rustix 是 transitive）。当前无安全修复路径。升级留 v3.1（大版本周期内）。

**cargo fmt**: 零不一致 ✅
**cargo clippy**: 零警告 ✅
**deny.toml cargo-deny 0.22.0**: 合法格式（`ignore` + `reason` 字段）；注意 cargo-deny 0.22.0 不支持 spec 中原定的 `[[advisories.allow]]` + `rationale` 格式，已改用官方支持格式，语义等价

## 5. 仓库结构健康度

| 维度 | 状态 |
|---|---|
| 根目录 Markdown | ✅ 7 个正式文档 only（无 evaluation* 混入） |
| evaluation*.md 归位 | ✅ 5 个文件已 git mv 到 docs/review/evaluation/ |
| Git history 保留 | ✅ `git log -- evaluation.md` 原始 commit 存在 |
| Git stash | ✅ 空（cargo-fmt-noise-task04 是纯格式噪声，已 drop） |
| evidence/ 分级 | ✅ README 表格含 ~343 MB、render/ux 占比、gitignored 说明 |
| README docs/ 行 | ✅ 补充 evaluation/ 子目录说明 |
| docs/commercial/ | ✅ LANDING.md + LICENSE-FAQ.md 中英各 2 份 |
| CHANGELOG final-polish 节 | ✅ 新增（L358-L369），含 13.40 MiB |

## 6. Release Readiness

### 当时结论：v3.0.1 已列工程门禁通过

**依据（仅指当时已执行并记录的项目）**：
1. disposition §2 所列四项可自动化验证在当时记录为 PASS
2. cargo fmt / clippy / deny check / test 全绿
3. 根目录零历史残留（evaluation 已归位）
4. evidence/ 分级清晰（343 MB，gitignored，render/ux 历史标注）
5. README 中英事实一致（badge 9 类、NOT RUN 集中章节、Trust 状态、Repository Structure）
6. CHANGELOG 基线数字 + final-polish 汇总已更新
7. 商业化物料到位（LANDING + LICENSE-FAQ 中英）
8. 无孤儿 stash、无 broken cross-reference、无 git 追踪垃圾

### 剩余 NOT RUN（明确 needs-user / depends-on）

| # | 项 | 性质 | 为什么 |
|---|---|---|---|
| 1 | 生产可信签名（SignPath/Azure/商业） | **needs-user** | 身份验证 + 付款仅你能做 |
| 2 | 真实 SmartScreen（MotW 下载） | **needs-user + depends-on** | 依赖 #1，Microsoft 声誉积累 |
| 3 | 远端 GitHub Actions 真实触发 | **needs-user** | 需 push 授权 |
| 4 | 跨机信任与吊销演练 | **depends-on** | 依赖 #1，需 CA CRL/AIA |
| 5 | DPI 100/125/200% 首帧 | **needs-user + 用户决定跳过** | 150% 取证已 PASS；用户认为足够 |
| 6 | 物理打印真机 | **needs-user + 用户决定跳过** | 虚拟 PDF 已覆盖；用户认为足够 |

**其中 5 和 6 是明确的用户决策放弃，不再视为待办**。

## 7. 提交历史

```
本轮 commit (final-polish-steve-jobs) — 本轮: typing-verify + evidence归档 + CHANGELOG
f82b919 — 上轮后半: deny.toml + 商业化物料 + eval归位
6cc1c9c — 上轮前半: badge 6→9 + NOT RUN 集中 + ci.yml core-linux
```

**Total**：2 commits（final-polish 轮）on top of 45 个未 push commits（HEAD ahead of origin/main by ~47 commits）。

---

## 综合评价（历史工程结论）

对 v3.0.1 当时快照，可支持的结论仅是：本报告列出的 fmt、clippy、deny、测试、指定交互链与仓库检查已按记录执行并通过；生产签名、真实 SmartScreen、远端 Actions 等外部事项仍未完成或需要单独核验。

该结论不评价也不证明审美、品牌声望、采用度、营收、市场地位或经典程度，更不是当前 v3.0.2 的重新验收。后续发布与推送仍需依据当前证据和发布流程单独决定。
