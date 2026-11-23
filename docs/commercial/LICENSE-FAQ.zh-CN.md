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

[github.com/31i45/veil](https://github.com/31i45/veil)。所有发布使用开发证书 `CN=Veil Development (Long-term)` 签名（SHA-1 指纹 `86C8793A44EF01EE1633BCF769244894326B4526`）。

### 签名 / 信任状态？

当前发布使用**自签名开发证书**。其他机器会显示未知发布者警告。完整细节与规划中的生产签名路径（SignPath Foundation — 符合条件的开源项目免费 OV，Azure Artifact Signing — 每月 US$9.99，商业 OV/EV — 每年 US$129 起）见仓库的「信任与验证状态」章节与 [evidence/renewal/production-signing-options.md](../../evidence/renewal/production-signing-options.md)。

### Veil 会回连服务器吗？

**不会。** 零遥测。没有崩溃报告。没有分析。没有静默更新。没有后台网络请求。永远。见 [PRIVACY.md](../../PRIVACY.md)。

### Windows SmartScreen 警告？

因为 Veil 当前使用自签名证书，SmartScreen 可能警告或拦截下载。受信任的生产证书会解决这个问题——相关流程正在积极评估中。

### 谁在维护 Veil？

一个人。所有发布、所有提交、所有签名。没有公司参与，没有风险投资，没有产品经理。
