# Veil

**单文件 Markdown 优先的 Windows 集成开发环境。纯离线。零遥测。你的思维、你的文字、你的数据。**

## 为什么是 Veil

Veil 为作家、思考者、研究者和工程师而生——想要无干扰写作，又不想放弃 Markdown 的全部能力。与其他写作工具不同，Veil 完全离线运行——没有云端，没有遥测，没有后台网络请求，没有静默更新。你的文字就留在你放它的地方。

## 核心

- **进化的 Markdown** — CommonMark 0.31.2 + GFM 扩展（表格 / 任务列表 / 删除线），实时增量语法高亮
- **单文件便携 exe** — ≈13.3 MiB，无需安装，无外部 DLL。可选的 per-user 安装包已提供
- **Mermaid 图表进程内渲染** — 没有外部网络调用用于图表渲染
- **极速** — 冷启动 145–190 ms（main → 首帧，5 次中位数），击键 p95 2.6 ms（21,000 块、300 样本），10 万字 150% DPI 滚动峰值 28.83 MB
- **零遥测** — 没有分析，没有崩溃报告，没有回调。永远
- **MIT 许可证** — 源码开放于 [github.com/31i45/veil](https://github.com/31i45/veil)

## 技术

- Rust 1.99.1 · Edition 2021
- 纯 Windows 11 x64 · winit + softbuffer 软件渲染 · QuickJS 嵌入式运行时
- veil-core（跨平台逻辑）· veil-ui（渲染）· veil-mermaid-offline（QuickJS + mermaid.js）· veil（产品二进制）

## 获取

[github.com/31i45/veil/releases](https://github.com/31i45/veil/releases)

SHA-256 验证脚本见根目录 README。当前使用自签名开发证书 `CN=Veil Development (Long-term)` 签名（SHA-1 指纹 `86C8793A44EF01EE1633BCF769244894326B4526`）。受信任的生产签名已规划——见仓库的「信任与验证状态」章节。

## 隐私

Veil 永远不会回连任何服务器。没有后台网络。没有静默更新。没有遥测。永远。

## 常见问题？

见 [LICENSE-FAQ.md](./LICENSE-FAQ.md)。技术问题？在 GitHub 提 issue。
