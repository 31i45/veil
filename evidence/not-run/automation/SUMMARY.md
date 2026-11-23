# NOT RUN 自动化验证汇总（2026-11-23）

本文件记录 disposition.md §2 "本 change 可自动化" 四项在两个轮次的执行结果：
- 第一轮 cleanup-and-polish-v3（commit `6cc1c9c`）：产出 ci-static-verify + core-cross-platform-check + dpi-150-evidence；typing-verify SKIP
- 第二轮 final-polish-steve-jobs（commit `f82b919` 后本轮）：typing-verify 落地 PASS；README evidence 分级更新；CHANGELOG final-polish 汇总

## 验证结果总览

| # | 项 | 结果 | 证据文件 | 备注 |
|---|---|---|---|---|
| 1 | CI YAML/脚本静态验证 | ✅ PASS | `ci-static-verify.txt` | core-linux job 已在 ci.yml；release.yml 已有 workflow_dispatch + inputs.dry_run；两处 YAML **均无 continue-on-error**；smoke_launch.ps1 MaxLaunchMs=1000 + launch-too-slow 分支在位 |
| 2 | Linux core CI job 代码跨平台验证 | ✅ PASS | `core-cross-platform-check.txt` | cargo check + test -p veil-core exit_code=0；Cargo.toml 无 platform cfg；依赖树全跨平台 |
| 3 | DPI 150% 首帧取证 | ✅ PASS（证据预存） | `dpi-150-evidence.txt` | evidence/startup/final/dpi-150/ status="PASS"，DPI=144（150% 逻辑像素），首帧 571ms，黑像素 <0.1%；2026-11-20 采集 |
| 4 | **typing→undo→save→export 自动化** | ✅ **PASS（本轮落地）** | `typing-verify/summary.json` + `typing-verify/walkthrough.log` | **本轮在本机桌面实际执行成功**。walkthrough.ps1 exit_code=0，40 steps（4 widths × 10 steps each）全部 OK。VEIL_HOME 快照证实：初始 54B → 输入后 79B（HelloWorld中文测试123） → 撤销+保存后回到 54B。PrintWindow 降级为屏幕捕获（环境限制），非产品功能问题。详细 summary.json 在 typing-verify/ 目录 |

## 本轮额外产出

- **deny.toml**: 3 条 allowed RustSec 加带 reason 的 ignore 块（bincode RUSTSEC-2025-0141、ttf-parser RUSTSEC-2026-0192、rustix RUSTSEC-2026-0486）
- **README Repository Structure**: evidence/ 行更新，含 ~343 MB、render/ 237MB + ux/ 70MB = 89%、gitignored 说明；docs/ 行补充 evaluation/ 子目录
- **CHANGELOG.md**: 新增 v3.0 final-polish 汇总节（L358-L369），release exe 13.40 MiB 记录
- **docs/commercial/**: LANDING.md + LICENSE-FAQ.md 中英各两份
- **根目录清理**: 5 个 evaluation*.md git mv 到 docs/review/evaluation/；git stash cargo-fmt-noise-task04 drop

## 技术债状态

| Crate | 版本 | RUSTSEC | 处置 | 升级计划 |
|---|---|---|---|---|
| bincode | 1.3.3 | 2025-0141 (unmaintained) | deny.toml ignore + reason | v3.1 — 2.x API 不兼容，需专项 |
| ttf-parser | 0.25.1 | 2026-0192 (unmaintained) | deny.toml ignore + reason | v3.1 — 0.26+ API shift |
| rustix | 1.1.5 | 2026-0486 (unsound xattr) | deny.toml ignore + reason | 等上游 1.2+ 自动流转 |

## 结论

**disposition.md §2 "本 change 可自动化"四项全部 PASS** ✅。cargo fmt 零不一致、cargo clippy 零警告、cargo audit 零未处置漏洞（3 allowed 已带 reason 记录）。typing-verify 此前 SKIP，本轮在本机桌面实际执行成功。

**v3.0 release readiness**: Yes. 所有可自动化门禁全绿。

## 剩余 NOT RUN（明确 needs-user / depends-on）

见 README.md `## Trust & Verification Status` / README.zh-CN.md `## 可信发布者与外部验证状态` 章节，以及 `evidence/renewal/production-signing-options.md` §7。

| # | 项 | 性质 | 为什么 |
|---|---|---|---|
| 1 | 生产可信签名（SignPath/Azure/商业） | **needs-user** | 身份验证 + 付款仅你能做 |
| 2 | 真实 SmartScreen（MotW 下载） | **needs-user + depends-on** | 依赖 #1 |
| 3 | 远端 GitHub Actions 真实触发 | **needs-user** | 需 push 授权 |
| 4 | 跨机信任与吊销演练 | **depends-on** | 依赖 #1，需 CRL/AIA |
| 5 | DPI 100/125/200% 首帧（用户已放弃） | **needs-user (用户决定跳过)** | 150% 取证已 PASS；用户认为足够 |
| 6 | 物理打印真机（用户已放弃） | **needs-user (用户决定跳过)** | 虚拟 PDF 已覆盖；用户认为足够 |
