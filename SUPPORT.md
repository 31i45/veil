# 支持说明

## 唯一官方来源

Veil 只从 GitHub Release 分发：

<https://github.com/31i45/veil/releases>

不要使用镜像站或他人重新上传的 exe。若你拿到的文件不是来自上述页面，请先停止使用。

## 下载后先核验 SHA-256

从**同一个 Release** 下载 `veil.exe` 与 `veil.exe.sha256`，放到同一目录，然后在 PowerShell 中执行：

```powershell
$exe = Join-Path $PWD 'veil.exe'
$sidecar = Join-Path $PWD 'veil.exe.sha256'
$expected = ((Get-Content -LiteralPath $sidecar -Raw).Trim() -split '\s+')[0]
$actual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw "SHA-256 不匹配：预期 $expected，实际 $actual" }
"SHA-256 核验通过：$actual"
```

不要拿其他 Release 的侧车文件核验当前 exe。SHA-256 一致只证明字节完整，不代表发布者信誉。

## 便携升级

- 直接用新版本的 `veil.exe` 覆盖旧 exe。**不要删除 `%APPDATA%\Veil`**：配置、会话与版本历史都在那里，会继续可用。
- 若设置过 `VEIL_HOME`，配置与数据位于该目录，升级同样不受影响。
- 若移动或替换了 exe 路径，Windows 的文件关联会失效：必要时在旧路径 `/unregister`、新路径 `/register`。
- 未知的新配置键不会导致启动失败。

## 常见问题入口

- 下载、核验、SmartScreen 与文件关联：[README.zh-CN.md](README.zh-CN.md)
- 已知限制（渲染与导出、Mermaid、PDF 断行等）：README 的「已知限制」一节
- 隐私、本地数据位置与网络能力：[PRIVACY.md](PRIVACY.md)
- 漏洞与安全报告：[SECURITY.md](SECURITY.md)
- 第三方组件与许可证：`THIRD-PARTY-LICENSES.md`（随发布包提供）
- 问题检索与提交：<https://github.com/31i45/veil/issues>

## 提交问题前请准备

1. **Veil 版本与构建提交**：关于对话框，或「帮助 → 导出诊断信息…」。
2. **诊断导出**：执行**帮助 → 导出诊断信息…**，把导出的文本附上。它包含版本、构建提交、操作系统与架构、DPI、配置摘要、功能开关与最近事件。
3. **复现步骤**：尽量最小化。如需附文档，请用不含隐私的**最小样例**。
4. **不要粘贴文档正文、剪贴板内容或完整文件路径**；诊断导出本身已排除这些字段。

崩溃时，报告会写入 `%APPDATA%\Veil\logs\`（或 `VEIL_HOME` 下），可将最近一份一并提供。

## 不要做的事

- 不要为运行 Veil 而关闭 SmartScreen 或杀毒软件。若被拦截，请先核对官方来源、SHA-256 与发布者信息，不一致就停止运行。
- 不要从第三方站点下载所谓「修复版」或「加速版」。

## 响应说明

这是一个小规模项目。我们会阅读并尽量处理每份报告，但**不承诺响应时限、修复时间或任何 SLA**。安全问题请按 [SECURITY.md](SECURITY.md) 走私密渠道，不要开公开 Issue。
