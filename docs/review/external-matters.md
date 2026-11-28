# 外部事项清单

> 维护人：Veil 项目主维护者
> 最后更新：2026-11-28（v3.1.0 发布轮：首次真实推送与 tag 触发）

本清单记录超出代码仓边界、需由真人/外部系统完成的 release 阻塞项，以及证书等长期维护计划。每项标注责任人、前置条件与当前状态。

## 发布阻塞项

| # | 事项 | 责任人 | 前置条件 | 当前状态 |
|---|---|---|---|---|
| 1 | **生产可信签名** — SignPath Foundation / Azure Artifact Signing / 商业 OV/EV 三选一 | 主维护者（身份验证 + 付费） | 身份验证完成；证书私钥以 Repository Secrets 形式注入 `VEIL_SIGNING_CERTIFICATE_BASE64` + `VEIL_SIGNING_CERTIFICATE_PASSWORD`；时间戳 URL 配置 `VEIL_SIGNING_TIMESTAMP_URL` | **NOT RUN** — 三条路径候选评估见本地 `evidence/renewal/production-signing-options.md` |
| 2 | **真实 SmartScreen / MotW 下载** — 从 Release 下载带 Mark-of-the-Web 的产物，验证 SmartScreen 信誉/警告/拦截行为 | 主维护者 + 全新 Windows VM | #1 完成（可信生产签名存在） | **NOT RUN** |
| 3 | **跨机信任与吊销演练** — 从非开发机下载签名产物，验证发布者身份、证书链、吊销检查 | 主维护者 + 至少一台全新 Windows 设备 | #1 完成（生产签名含 CRL/AIA）；或先在可信生产环境测试 | **NOT RUN** — 自签名证书无公共同名吊销机制 |
| 4 | **远端 GitHub Actions 真实触发** — 推送 `v*` tag 后验证 CI 全链路（含 Linux core job + Windows release job + dry_run 默认值不发布） | 主维护者 | `v*` tag push | **PASS（2026-11-28）** — run `46089558590`：`core-linux` 与 `release` 全部成功，Release 已发布 4 件资产（`veil.exe` / `.sha256` / `veil-setup-3.1.0.exe` / `.sha256`）。历史：首轮 tag 因字族测试的环境字体差异失败，修复后重打 v3.1.0 |
| 5 | **外部安全审计** — 独立第三方对远程图片抓取、Mermaid 渲染 QuickJS 沙箱、资源上限边界做代码级审计 + 恶意构造测试 | 外部安全团队 | 生产签名 + 发布就绪（确保审计对象即发布对象） | **NOT RUN** — 历史自签名回环/QuickJS 中断边界已自证，但无独立第三方背书 |
| 6 | ~~**NVDA / 其他 AT 用户测试**~~ — 功能已移除 | — | — | **不适用** — 2026-11-25 产品决定移除 AccessKit 无障碍支持（最终移除提交 `3a6d72d`）：真实 Narrator 只能朗读窗口/文档标题，无法进入菜单与正文。原前置条件（AccessKit handlers、中文语音包）一并关闭，详见 `.trae/specs/remediate-product-trust-and-accessibility/review.md` Review 3 |
| 7 | **多设备 / 物理打印** — 至少两台物理打印驱动 + 一台虚拟化驱动，验证分页、中文字体、页边距、纸张尺寸 | 主维护者 + 物理/虚拟化打印机环境 | 打印机可用；`platform::print` + `pdf_layout` 已交付 | **NOT RUN** — GDI 打印路径代码已存在，但本轮无物理设备验证 |

## 不在本清单中（已完成或已降为文档声明）

- **历史自签名流水线验证**：2026-11-23 本机自签名证书 Authenticode + RFC3161 时间戳 + 验签已完成（仅用于验证流水线，不是生产信任锚）
- **范围受限本机抓包**：`pktmon` 历史记录已在 PRIVACY.md 中声明，不宣称全场景零出站
- **代码级单元/integration tests**：属于仓库内部责任，已在 `cargo test --workspace --release -- --test-threads=1` 中覆盖

## 签名与证书维护计划

| 项 | 现状 | 触发条件 | 到期前动作 |
|---|---|---|---|
| 自签名开发证书 | `CN=Veil Development (Long-term)`，SHA-1 `86C8793A44EF01EE1633BCF769244894326B4526`，NotAfter **2031-11-23T05:06:11Z**；仅本机当前用户根存储信任 | 每次 `v*` tag 发布（secrets 已配置时自动签名 + RFC3161 验签；未配置则明确记录 UNSIGNED） | 在 **2031-08-24** 前二选一：续期自签名（仅限本机/内测）或切换到生产签名 |
| 生产签名（SignPath / Azure / 商业 OV/EV） | NOT RUN；三方案评估见本地 `evidence/renewal/production-signing-options.md` | SmartScreen/SAC 成为主要投诉，或需要对外分发 | 提前完成身份验证与付费；把 PFX Base64 与密码写入 Repository Secrets，时间戳 URL 写入 Variables |
| 证书轮换记录 | 指纹同时出现在 README 中英、PRIVACY.md | 任何一次证书更换 | 证书轮换必须单独 commit，并同步更新双 README、PRIVACY 与 `docs/commercial/`，再跑一次签名冒烟 |

> 过期证书不得继续用于发布：`release.yml` 的签名步骤会因验签失败而中止发布，而不是静默产出过期签名的产物。

## 待办分层（P1 / P2 / P3）

| 级别 | 事项 | 为什么还没做 | 完成判据 |
|---|---|---|---|
| P1 | DPI 100/125/200 逐档点按、拖选、双击选词、三击选段，核对光标落点 ≤1 字符 | 用户已人工执行（2026-11-28） | **用户确认 PASS（正常）**；未落盘 jsonl，最终判读以用户口径为准 |
| P1 | 微软五笔真机六步 | 用户决定不做（多人不用五笔）；微软拼音六步已 PASS | **不做** |
| P2 | 失焦提交原始串的取舍 | 用户已拍板：保持现状（不丢字、不重复） | **保持现状** |
| P2 | 实体打印机冒烟 | 用户决定不做：PDF 本就是打印产物，Print-to-PDF 已覆盖应用侧分页 | **不做** |
| P3 | 真实断网抓包归因 | 需要断网环境；PRIVACY 已按「范围受限」声明 | 断网 + `pktmon` 复验，更新 PRIVACY |
| P3 | 无 CJK 字体系统、只读导出目标 | 需要专用环境 | 两个环境的导出/打印冒烟记录 |
| P3 | 外部第三方安全审计 | 需要预算与发布对象冻结 | 审计报告 + 恶意构造用例闭环 |
| 延后 | GPU / 移动端 / PWA / 插件 / 协作 / 合并单元格 | 已在 `docs/roadmap/` 与 README 非目标登记「为什么不做 + 何时重开」 | 触发条件出现时再立项 |
