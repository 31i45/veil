# Veil

**单文件 Windows Markdown 编辑器。本地优先。零遥测。你的思维、你的文字、你的数据。**

## 为什么是 Veil

Veil 为作家、思考者、研究者和工程师而生——希望减少干扰，同时保留对 Markdown 源码的控制。Veil 默认将文档保留在本地：无云同步、遥测或后台更新检查。远程图片是独立的持久开关；开启后，打开含远程图片的文档会按需发起请求。外部链接和 mermaid.live 只在用户显式操作时打开。

## 核心

- **边界明确的 Markdown** — CommonMark 0.31.2 核心 + GFM 子集（表格 / 任务列表 / 删除线 / 自动链接），实时增量语法高亮。这不代表完整兼容所有 Markdown；独立测试集、一致性维度与已知限制见[根目录 README](../../README.zh-CN.md#历史测量目标值与结果)
- **单文件便携 exe** — 无需单独安装应用运行时或安装包，但仍使用 Windows 系统组件与系统 DLL。可选的 per-user 安装包已提供
- **Mermaid 图表进程内渲染** — 图表渲染本身不发起外部网络请求
- **证据绑定构建** — 历史性能测量在根目录 README 中保留各自日期、构建与方法；本轮文档纠偏未对 v3.1.1 复测
- **零遥测上传** — 无分析或遥测上传。崩溃与诊断报告在本地生成，不会自动上传
- **MIT 许可证** — 源码开放于 [github.com/31i45/veil](https://github.com/31i45/veil)

## 技术

- Rust 1.99.1 · Edition 2021
- 纯 Windows 10/11 x64 · winit + softbuffer 软件渲染 · QuickJS 嵌入式运行时
- veil-core（跨平台逻辑）· veil-ui（渲染）· veil-mermaid-offline（QuickJS + mermaid.js）· veil（产品二进制）

## 获取

[github.com/31i45/veil/releases](https://github.com/31i45/veil/releases)

SHA-256 验证脚本见根目录 README。当前使用自签名开发证书 `CN=Veil Development (Long-term)` 签名（SHA-1 指纹 `86C8793A44EF01EE1633BCF769244894326B4526`）。受信任的生产签名已规划——见仓库的「信任与验证状态」章节。

## 隐私

Veil 不包含遥测、云同步或后台更新检查。**帮助 → 检查更新**只在浏览器中打开官方 Release 页。远程图片默认关闭，开关状态会持久保存：开启后，之后打开含远程图片的文档可按需发起请求，不再逐次提示。外部链接和 mermaid.live 只在用户显式操作时打开。

## 常见问题？

见 [LICENSE-FAQ.md](./LICENSE-FAQ.md)。技术问题？在 GitHub 提 issue。
