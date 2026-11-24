# Veil — 许可证与信任常见问题

### Veil 使用什么许可证？

MIT。见 [LICENSE](../../LICENSE)。你可以在商业中使用 Veil、修改它、再分发它，甚至在 MIT 条款下再授权它。没有公司实体或 CLA——整个项目由一位维护者独立完成。

### 为什么选 MIT？

MIT 是最宽松、最广泛被理解的开源许可证。Veil 从一款个人工具成长为产品。维护者相信你的文字属于你，你的工具不应该把它们扣为人质。MIT 保持简单：你想做什么就做什么，只要不去告任何人。

### 在哪里能看到第三方许可证列表？

完整的第三方许可证清单（Rust crate、Mermaid.js、linkedom、QuickJS 捆绑资源）位于根目录 [THIRD-PARTY-LICENSES.md](../../THIRD-PARTY-LICENSES.md)。每次发布时通过 `tools/third_party_notices.ps1` 更新。

### Veil 可以免费用于商业吗？

可以。MIT 不限制商业使用。你可以把 Veil 作为商业产品的一部分出货、为它收费、或在它之上构建。

### 源码在哪里？

[github.com/31i45/veil](https://github.com/31i45/veil)。当前便携 `veil.exe` 与 per-user 安装包 `veil-setup-<version>.exe` 使用开发证书 `CN=Veil Development (Long-term)` 签名（SHA-1 指纹 `86C8793A44EF01EE1633BCF769244894326B4526`）。

### 签名 / 信任状态？

当前便携 exe 与安装包使用**自签名开发证书**，该证书仅在当前开发机上被信任。其他机器会显示未知发布者警告。可信生产签名与真实 SmartScreen 验证仍为 **NOT RUN**。候选生产签名路径（SignPath Foundation — 符合条件的开源项目免费 OV，Azure Artifact Signing — 每月 US$9.99，商业 OV/EV — 每年 US$129 起）见仓库的「信任与验证状态」章节。

### Veil 会回连服务器吗？

Veil 不包含遥测或分析，也没有后台更新检查，但不能据此表述为永不联网。崩溃与诊断报告在本地生成，不会自动上传；只有你主动提交给支持渠道时，相关报告才会离开本机。**帮助 → 检查更新**只在系统浏览器中打开官方 Release 页。远程图片默认关闭；持久开启该设置后，之后打开含远程图片的文档会按需自动请求，不再逐次提示。见 [PRIVACY.md](../../PRIVACY.md)。

### Windows SmartScreen 警告？

因为 Veil 当前使用自签名证书，SmartScreen 可能警告或拦截下载。受信任的生产证书不保证 SmartScreen 不再警告：其信誉系统需要随时间积累，且真实 Mark-of-the-Web 下载验证仍为 **NOT RUN**。

### 谁在维护 Veil？

一个人。所有发布、所有提交、所有签名。没有公司参与，没有风险投资，没有产品经理。
