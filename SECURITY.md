# 安全策略

## 支持版本

Veil 以 **3.x** 系列为当前支持版本。安全修复进入最新 3.x 发布；更早的版本不再单独维护。请先确认问题在最新官方 Release 上仍可复现：

- 唯一官方来源：<https://github.com/31i45/veil/releases>
- 请同时下载 `veil.exe` 与同一 Release 的 `veil.exe.sha256` 并核验，步骤见 [SUPPORT.md](SUPPORT.md)。

## 报告漏洞

请**不要**通过公开 Issue 披露未修复的安全问题。使用 GitHub 私密漏洞报告：

<https://github.com/31i45/veil/security/advisories/new>

这是本仓库唯一的私密报告渠道。请勿使用镜像站、论坛或第三方邮箱转交。

## 报告应包含的信息

- 受影响的 Veil 版本（关于对话框或「帮助 → 导出诊断信息…」中的版本与构建提交）。
- Windows 版本与架构、是否启用远程图片。
- 复现步骤，尽量最小化；涉及文档时请用**构造的最小样例**，不要附上你的真实文档正文。
- 实际结果与预期结果。
- 如涉及崩溃，请附「帮助 → 导出诊断信息…」导出的诊断文本（其中不含文档正文、剪贴板与完整文件路径）。

## 范围

属于安全问题的例子：

- 通过恶意 Markdown 文档触发的内存安全问题、越界或拒绝服务。
- 链接协议白名单绕过，或在未经用户操作时打开外部程序 / 发起网络请求。
- 远程图片抓取中的 SSRF 类问题（绕过 localhost / 回环 / 私有 / 链路本地地址限制，或绕过 https→http 降级拒绝）。
- 导出（HTML / PDF）过程中引入可执行内容或路径穿越。
- 崩溃报告或诊断导出泄露文档正文、剪贴板或完整文件路径。

不属于范围的例子：

- 需要在本机已具备管理员权限或已能改配置文件的前提下才成立的问题。
- 自签名开发证书或新证书触发的 SmartScreen 提示（这是分发信誉问题，见 [README](README.md)）。当前产物使用自签名开发证书 `CN=Veil Development (Long-term)`（SHA-1 指纹 `86C8793A44EF01EE1633BCF769244894326B4526`，NotAfter `2031-11-23T05:06:11Z`）签名并加 RFC3161 时间戳，仅用于验证签名流水线；显式信任仅限本开发机当前用户，**不是跨机或生产信任锚**，可信生产签名仍为 **NOT RUN**。
- 第三方依赖自身的上游问题（欢迎告知，但修复以上游为准；依赖清单见 `THIRD-PARTY-LICENSES.md`）。
- 纯主观的界面 / 文案问题。

## 处理方式

- 确认后会在后续 3.x 版本中修复，并在 Release 说明中致谢（除非你要求匿名）。
- 不承诺固定的响应时限或修复 SLA；这是一个小规模项目，但会认真处理每份报告。

## 远程图片与 Mermaid 的当前验证状态

以下结论仅对应 **2026-11-24 的当前工作树**及本机 Windows 测试，不代表独立安全审计。

### 已复现并修复（限已验证边界）

- **远程图片回环绕过**：修复前，`http://localhost.:<端口>/...` 能取回本地夹具返回的 JPEG；夹具记录的实际目标与客户端均为 `127.0.0.1`。现在入口会规范化尾部根点并拒绝该主机，同时禁用代理，并在每个 WinHTTP 请求收到响应后查询实际远端地址；回环、私网、链路本地等受限 peer、地址查询失败、未知地址族及异常端口均失败关闭，不再把响应交给解码器。
- **QuickJS 单次执行不受旧轮询 deadline 控制**：修复前，直接执行 `while (true) {}` 的子进程在 deadline 后仍不退出，只能由父进程终止。现在 `rquickjs 0.9.0` runtime 设置了 interrupt handler，与每次渲染共享 deadline。病态无限循环和持续 Promise 微任务两个子进程回归均能退出，且中断后 runtime 可继续求值。生产上限仍为 20 秒；回归把同一 deadline 缩短为 200ms，未实际等待完整 20 秒。

### 未复现但有测试

- URL 入口回归覆盖 `localhost.`、数字/十六进制及点分数字形式、回环/私网/链路本地等字面量；地址策略回归覆盖实际 peer 可能呈现的 IPv4、IPv6 和 IPv4-mapped 受限地址。除上述 `localhost.` 回环连接外，这些是过滤或地址策略测试，不是对应地址的真实联网复现；点分十六进制主机尤其未被证明会由 WinHTTP 成功解析并连接。
- 重定向回归覆盖绝对/相对地址、受限字面量和 https→http 降级；每个实际 hop 都经过相同的 peer 检查。尚未以此宣称真实“公网首跳 → 解析后受限次跳”链路已经复现。
- 未复现从恶意 Mermaid 图源码直接注入任意 JavaScript 的路径。`securityLevel: 'loose'` 配置字符串本身不被视为漏洞证据；上述 QuickJS 测试直接操作执行上下文，证明的是执行中断边界，不是 Mermaid 源码注入能力。

### 仍未验证与现有限制

- **实际 peer 核验不是连接前隔离。** WinHTTP 在 `WinHttpReceiveResponse` 返回后才提供本实现查询的连接信息，因此受限目标的 TCP/TLS 建连及 HTTP 请求可能已经发生；当前处置保证响应失败关闭，不保证对受限地址“完全无网络接触”。需要严格连接前保证时，应改用能把固定且已校验的解析地址与原始 TLS 主机名/证书校验绑定的连接栈，或另行实现仅允许公网 IP 字面量的受限模式；这些替代目前均未实现。在此之前，处理不可信文档应保持远程图片关闭。
- 真实系统 DNS 切换/rebinding、解析后私网或链路本地的实际 socket 链路、完整公网到受限地址重定向链路仍为 **NOT RUN**。
- 真实 TLS 握手、SNI、证书主机名匹配及错误证书失败关闭未做本轮集成实测。实现仍把原始 URL 主机名交给 WinHTTP，未设置忽略证书错误；这不替代上述测试。
- 为使实际 peer 指向目标而非代理，远程图片会话现在使用直连，不再沿用系统代理；依赖代理才能联网的环境会因此无法抓取远程图片。
- **资源上限边界已实现但未见恶意实测**：远程图片单张响应体上限 20 MiB + 会话内存缓存总上限 200 MiB（LRU 逐出，不落盘）、QuickJS 引擎 512 MiB heap 上限 + 20s 渲染中断、视口驱动回收（layouts/rasters/tables 剪到可视范围）。这些是代码实现的防御性边界，未见恶意构造大文档或超长 Mermaid 源码触发拒绝服务的实测。

## 威胁模型

范围：**用户打开的 `.md` 及其中的原始 HTML、图片引用、链接、剪贴板片段一律视为不可信输入**。
信任边界：用户通过系统对话框 / CLI 亲自选择的文件路径属于可信意图；文档内容本身不可信。
以下结论对应 2026-11-25 的工作树，回归测试见 `crates/veil-ui/src/security_tests.rs`。

### 1. HTML 注入（原始 HTML / 事件属性 / `javascript:` 链接）

- **攻击面**：Markdown 正文里的块级与行内 HTML 直接进入导出成品；粘贴的 HTML 片段进入文档。
- **入口**：解析层 `crates/veil-core/src/syntax/inline.rs`（属性收集）；导出侧
  `crates/veil/src/export.rs`（`sanitize_block_html`、`sanitize_block_attrs`、
  `strip_unsafe_url_attrs`、`sanitize_html_urls`）；粘贴 `crates/veil-ui/src/html.rs`。
- **当前防护**：
  - `on*` 事件属性一律忽略（`inline.rs` 中 `an.starts_with("on")` 直接 `continue`）。
  - `style` 经受 `veil_core::sanitize_style` 白名单：只保留 7 个属性
    （`color`/`background-color`/`font-weight`/`font-style`/`text-decoration`/
    `font-size`/`font-family`），值中出现 `url(`、`expression`、`javascript:`、
    `behavior`、`position`、`z-index`、`content` 即整条声明丢弃。
  - 危险标签整段剥除：`veil_core::should_strip_html` 覆盖
    `script/style/iframe/object/embed/form/textarea`；只保留
    `HTML_INLINE_WHITELIST` 内标签，其余标签转义为文本。
  - 链接目标统一走 `veil_core::link_url_is_safe`（白名单 `http/https/mailto/tel`，
    先还原实体、百分号与控制字符混淆）；`javascript:`/`data:`/`file:`/`vbscript:`
    只保留文字，不写 `href`/`src`。
- **状态：`已有防护`**。回归测试：
  `security_tests::raw_html_dangerous_tags_are_stripped_not_escaped_through`、
  `security_tests::pasted_html_events_scripts_and_unsafe_images_are_neutralized`。

### 2. `file://` 图片（本地图片加载）

- **攻击面**：文档中的 `![...]()` 或 `<img src>` 让渲染器读取本地文件。
- **入口**：路径判定 `crates/veil-ui/src/paint.rs::local_image_path` /
  `is_remote_image_url`；实际解码 `crates/veil/src/lib.rs::FsImages::load`（PNG 或 WIC）。
- **当前防护**：带 scheme 的 URL（`file:`、`http(s):`、`data:`、`javascript:` 等）
  一律不按本地路径解析（scheme 长度 ≥2 且全为字母 → `None`）；相对路径以文档所在目录
  为基准，百分号编码（`%E4%B8%AD`、`%20`）先按 UTF-8 解码；远程图片另由
  `allow_remote_images` 开关控制（默认关闭）。
- **已知放宽（有意行为）**：**无 scheme 的绝对路径**（如 `C:\pics\a.png`）与包含 `..`
  的相对路径仍会被加载，以便作者引用本机图片；读取只发生在渲染端，内容不会外传。
  这也意味着恶意文档可以让用户在本机看到任意可解码图片（见第 3 节）。
- **状态：`已有防护`**（scheme 级）。回归测试：
  `security_tests::file_scheme_images_are_never_loaded`。

### 3. 路径穿越

- **打开 / 另存**：路径来自系统对话框或 CLI（`FileDialog`、`Doc.path`），`..` 由操作系统
  按用户意图解析；不存在「应用替用户拼接路径」的信任边界。
  **状态：`不适用`**（无特权提升面）。
- **粘贴图片资产写入**：目录名由 `veil/src/lib.rs::asset_dir_name` → `safe_stem` 生成
  （仅保留字母数字/`-`/`_`，空名回退 `document`），文件名固定 `image-<n>.png`；
  `..` 无法构成目录项。另存迁移用 `path_is_within` + `lexical_absolute` 拒绝让 `..`
  把资产写出新文档目录。
  **状态：`已有防护`**。测试位于产品层：`asset_dir_name_and_stem_are_sanitized`、
  `migrate_relative_assets_copies_to_new_doc_directory`。
- **图片引用读取**：`local_image_path` 不消解也不拒绝 `..`，
  `../../Users/secret.png` 会解析到文档目录之外。
  **状态：`新增测试`**（缺口以 `#[ignore]` 锁定：
  `security_tests::image_reference_dotdot_is_contained_to_document_dir`）。影响仅为
  渲染端显示本地图片，没有外传通道，属低危设计取舍；最小复现与建议见
  `evidence/verify/qq-task6/findings.md`。

### 4. 导出 HTML 的脚本注入

- **攻击面**：导出的 `.html` 被浏览器打开时执行原文档携带的脚本。
- **入口**：`crates/veil/src/export.rs::html_of_doc` 与 `HtmlExporter::export_html`。
- **当前防护**：
  - 模板 `head()` 只写静态 `<style>`，不含 `<script>`、`@import` 或任何 http(s) 资源
    （成品级断言 `export_has_no_remote_resources`）。
  - 正文逐块净化：块级原始 HTML 走 `sanitize_block_html`，成品再走 `sanitize_html_urls`
    （删除 `on*`、`srcdoc`/`srcset`/`ping` 与非白名单的
    `href/src/xlink:href/action/...`）。
  - 远程图片 `src` 删除；本地图片按魔数只内嵌 PNG/JPEG/GIF/WebP/BMP 位图 data URI，
    SVG 等可携带脚本的文本格式拒绝（`decide_image_src` / `is_data_image`）。
  - 内嵌的 Mermaid SVG 同样经过上述成品净化（`<script>` 会被 `should_strip_html`
    整段剥除）；`<style>` 内容在净化后由 `restore_mermaid_styles` 回填，其安全性依赖
    Mermaid 不把图源码注入主题 CSS（本轮未做对抗性实测）。
- **状态：`已有防护`**。共享过滤原语回归测试：
  `security_tests::export_url_and_style_filters_block_script_vectors`；成品级断言在
  `crates/veil/src/export.rs::tests`（`block_html_strips_scripts_styles_and_embeds`、
  `dangerous_link_schemes_are_stripped_from_export`、
  `mermaid_style_restore_keeps_user_html_sanitization`）。

### 5. 剪贴板粘贴

- **攻击面**：从浏览器 / Word 复制到剪贴板的 HTML 片段。
- **入口**：`crates/veil-ui/src/interaction.rs::paste_clipboard` →
  `Clipboard::get_html` → `crates/veil-ui/src/html.rs::html_to_markdown`。
- **当前防护**：`script`/`style` 的原始文本整体丢弃；未知标签递归取子节点（属性不参与
  输出，只读取 `src`/`alt`/`href`）；`img`/`a` 的目标经 `veil_core::link_url_is_safe`，
  危险 scheme 只保留 alt 文字；最终产物是 Markdown 文本而非 HTML，不进入任何执行上下文。
- **状态：`已有防护`**。回归测试：
  `security_tests::pasted_html_events_scripts_and_unsafe_images_are_neutralized`、
  `security_tests::pasted_dangerous_links_are_degraded_to_text`。

## 依赖审计

- **`cargo tree --depth 1`**：产品层 `veil` 直接依赖 11 个外部 crate（arboard、base64、
  latex-rust、notify、pdf-writer、png、pulldown-cmark、resvg、subsetter、tiny-skia-path、
  ttf-parser）+ 3 个本地 crate（veil-core / veil-ui / veil-mermaid-offline），与代码引用的
  端口实现一一对应，没有多余的直接依赖。
- **`deny.toml` 现状**：`[advisories]` 含三条已评审豁免——bincode 1.3.3
  `RUSTSEC-2025-0141`、ttf-parser 0.25.1 `RUSTSEC-2026-0192`、rustix 1.1.5
  `RUSTSEC-2026-0486`（均写明理由与非用户面路径）；许可证为窄白名单
  （MIT、Apache-2.0、Apache-2.0 WITH LLVM-exception、BSD-2/3-Clause、ISC、Zlib、
  Unicode-3.0、BSL-1.0、CC0-1.0，`confidence-threshold = 0.93`）；
  `[bans]` 禁止新通配版本；`[sources]` 拒绝未知 registry / git。
- **本轮无新增依赖**：改动仅限 `SECURITY.md`、`crates/veil-ui/src/security_tests.rs` 与
  `evidence/verify/qq-task6/`；`Cargo.toml` / `Cargo.lock` 未变。

## 给用户的建议

- 只从官方 Release 下载，并核验 SHA-256。
- **不要**为了运行 Veil 而关闭 SmartScreen、杀毒软件或其他 Windows 安全功能。若系统给出警告，先核对来源、摘要与发布者，不一致就停止运行。
- 不需要管理员权限即可运行 Veil；请勿以管理员身份长期运行。
- 在不需要时保持「视图 → 允许远程图片」关闭（默认即为关闭）。
