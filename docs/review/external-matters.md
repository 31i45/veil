# 外部事项清单

> 维护人：Veil 项目主维护者
> 最后更新：2026-11-24（Task 10 口径终审同步）

本清单记录超出代码仓边界、需由真人/外部系统完成的 release 阻塞项。每项标注责任人、前置条件与当前状态。

| # | 事项 | 责任人 | 前置条件 | 当前状态 |
|---|---|---|---|---|
| 1 | **生产可信签名** — SignPath Foundation / Azure Artifact Signing / 商业 OV/EV 三选一 | 主维护者（身份验证 + 付费） | 身份验证完成；证书私钥以 Repository Secrets 形式注入 `VEIL_SIGNING_CERTIFICATE_BASE64` + `VEIL_SIGNING_CERTIFICATE_PASSWORD`；时间戳 URL 配置 `VEIL_SIGNING_TIMESTAMP_URL` | **NOT RUN** — 三条路径候选评估见本地 `evidence/renewal/production-signing-options.md` |
| 2 | **真实 SmartScreen / MotW 下载** — 从 Release 下载带 Mark-of-the-Web 的产物，验证 SmartScreen 信誉/警告/拦截行为 | 主维护者 + 全新 Windows VM | #1 完成（可信生产签名存在） | **NOT RUN** |
| 3 | **跨机信任与吊销演练** — 从非开发机下载签名产物，验证发布者身份、证书链、吊销检查 | 主维护者 + 至少一台全新 Windows 设备 | #1 完成（生产签名含 CRL/AIA）；或先在可信生产环境测试 | **NOT RUN** — 自签名证书无公共同名吊销机制 |
| 4 | **远端 GitHub Actions 真实触发** — 推送 `v*` tag 后验证 CI 全链路（含 Linux core job + Windows release job + dry_run 默认值不发布） | 主维护者 | `v*` tag push | **NOT RUN here** — 静态工作流验证已完成，当前 commit 未触发远端 |
| 5 | **外部安全审计** — 独立第三方对远程图片抓取、Mermaid 渲染 QuickJS 沙箱、资源上限边界做代码级审计 + 恶意构造测试 | 外部安全团队 | 生产签名 + 发布就绪（确保审计对象即发布对象） | **NOT RUN** — 历史自签名回环/QuickJS 中断边界已自证，但无独立第三方背书 |
| 6 | ~~**NVDA / 其他 AT 用户测试**~~ — 功能已移除 | — | — | **不适用** — 2026-11-25 产品决定移除 AccessKit 无障碍支持（commit `bd3b03e`）：真实 Narrator 只能朗读窗口/文档标题，无法进入菜单与正文。原前置条件（AccessKit handlers、中文语音包）一并关闭，详见 `.trae/specs/remediate-product-trust-and-accessibility/review.md` Review 3 |
| 7 | **多设备 / 物理打印** — 至少两台物理打印驱动 + 一台虚拟化驱动，验证分页、中文字体、页边距、纸张尺寸 | 主维护者 + 物理/虚拟化打印机环境 | 打印机可用；`platform::print` + `pdf_layout` 已交付 | **NOT RUN** — GDI 打印路径代码已存在，但本轮无物理设备验证 |

## 不在本清单中（已完成或已降为文档声明）

- **历史自签名流水线验证**：2026-11-23 本机自签名证书 Authenticode + RFC3161 时间戳 + 验签已完成（仅用于验证流水线，不是生产信任锚）
- **范围受限本机抓包**：`pktmon` 历史记录已在 PRIVACY.md 中声明，不宣称全场景零出站
- **代码级单元/integration tests**：属于仓库内部责任，已在 `cargo test --workspace --release -- --test-threads=1` 中覆盖（234 passed + docs_policy 6 passed）
