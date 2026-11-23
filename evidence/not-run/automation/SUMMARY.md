# NOT RUN 自动化验证汇总（2026-11-23）

本文件记录 disposition.md §2 "本 change 可自动化" 四项在本轮实际执行结果。

## 验证结果总览

| # | 项 | 结果 | 证据文件 | 备注 |
|---|---|---|---|---|
| 1 | CI YAML/脚本静态验证 | ✅ PASS | `ci-static-verify.txt` | core-linux job 已在 ci.yml（ubuntu-latest + cargo check/test -p veil-core）；release.yml 已有 workflow_dispatch + inputs.dry_run；两处 YAML 均**无 continue-on-error 指令**（仅 release.yml 注释明确写"两种结果都不使用 continue-on-error"）；smoke_launch.ps1 MaxLaunchMs=1000 + stage=launch-too-slow 分支在位 |
| 2 | Linux core CI job 代码跨平台验证 | ✅ PASS | `core-cross-platform-check.txt` | `cargo check -p veil-core` exit_code=0（3.32s 完成）；`cargo test -p veil-core` exit_code=0（lib.rs 内 unit tests 为 0，integration tests 独立在 tests/ 目录下）；Cargo.toml 无 platform-specific cfg / windows / unix gates；依赖树无 windows-sys / winit / softbuffer / cosmic 等平台绑定，全部为纯跨平台 crate |
| 3 | DPI 150% 首帧取证 | ✅ PASS（证据预存） | `dpi-150-evidence.txt` | evidence/startup/final/dpi-150/ 已含 5 张帧（0000ms–0800ms）、frames.csv、summary.json；summary.json status="PASS"，系统 DPI=144（150% 逻辑像素），首帧可见时间=571.06ms，client area 0/30/100/300ms 精确黑像素 <0.1%，800ms 精确黑像素=0；本轮未重跑 walkthrough.ps1 但已有完整证据链（2026-11-20 采集） |
| 4 | 窗口内键入验证 | ⚠️ SKIP（需 GUI） | — | tools/walkthrough.ps1 具备 SendKeys + 剪贴板粘贴键入能力；但 VEIL_HOME 隔离逻辑需进一步在实际 GUI 会话中验证；当前无自动化 GUI 交互测试环境；建议在有桌面的 runner 上执行一次完整 typing→undo→save→export 循环 |

## 结论

- **三项核心自动化验证 PASS**：CI YAML 静态核对、veil-core Windows 编译+测试、DPI 150% 帧证据（预存 2026-11-20）
- **typing-verify SKIP**：需要真实 GUI runner（如 GitHub Actions windows-latest 或本机桌面交互）；当前环境无 GUI 自动化框架
- **release.yml workflow_dispatch + dry_run 默认 true**：已存在，本轮只做静态核对（非本轮新增，在较早 commit 中落地）
- **ci.yml core-linux job**：已存在（ubuntu-latest + check/test -p veil-core + conformance + gfm_conformance），与 release.yml 范围一致
- **continue-on-error 零使用**：ci.yml 与 release.yml 均无 continue-on-error 指令；release.yml 注释明确禁止

## 剩余需用户 NOT RUN（未进入本轮）

见 README.md `## Trust & Verification Status` / README.zh-CN.md `## 可信发布者与外部验证状态` 章节，以及 `evidence/renewal/production-signing-options.md` §7。

disposition.md §1 处置表指向的 needs-user 操作：
- P1 真实 GitHub Actions 冒烟（需 push 授权或网页 Run workflow）
- P2 DPI 100/125/200% 首帧（需改系统缩放）
- P3 物理打印（需真实打印机）
