# Veil 人工测试协议（v3.0.0 · Windows 11 x64）

产物：`release-bundle\veil-v3.0.0\veil.exe` 或 `target\release\veil.exe`，**14,012,928 B（13.36 MiB）**，SHA-256 `ff23c4d9…dd25d0`。先核对：

```powershell
$exe = 'release-bundle\veil-v3.0.0\veil.exe'
(Get-Item $exe).Length; Get-FileHash $exe -Algorithm SHA256
```

**隔离**：每个会话设独立 `VEIL_HOME`，不打开真实文档；测完删除。崩溃恢复必须**不带文件参数**启动。

```powershell
$env:VEIL_HOME = "$env:TEMP\veil-manual"; New-Item -ItemType Directory $env:VEIL_HOME -Force | Out-Null
Start-Process .\release-bundle\veil-v3.0.0\veil.exe   # 结束后 Remove-Item Env:VEIL_HOME
```

**状态**：`自动`＝脚本可判 PASS/FAIL；`部分自动`＝脚本操作/采集，仍需人眼或改系统设置；`必须人工`＝不得以脚本代替。

**诚实边界**：已自动覆盖启动出窗、英文键入与撤销、保存、HTML 导出、20k 击键 p95、150% 首帧。中文 IME 候选窗、四档 DPI、实体打印、抓包、无 CJK 字体与只读目标等未验证项的统一定义与状态以 README「已知限制」为准，本文不重复维护；本记录中未做的项写 `NOT RUN`，不写 PASS。

**失败记录**：整窗截图（含时间）、`walkthrough.log`、产物 SHA、DPI/主题/输入法/路径、复现步骤。

以下命令在仓库根目录同一 PowerShell 会话执行（smoke 前须关闭所有 `veil` 进程）：

```powershell
pwsh -NoProfile -File tools/smoke_launch.ps1 -ExePath $exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000
pwsh -NoProfile -File tools/walkthrough.ps1 -Exe $exe -EvidenceMode input   -ShotDir evidence\manual\input
pwsh -NoProfile -File tools/walkthrough.ps1 -Exe $exe -EvidenceMode startup -ShotDir evidence\manual\startup
pwsh -NoProfile -File tools/walkthrough.ps1 -Exe $exe -EvidenceMode d1 -ColdRuns 10 -ShotDir evidence\manual\d1
```

## A 首跑

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| A1 | 干净 `VEIL_HOME` | 无参数启动；再 `Ctrl+N` | 示例页「欢迎使用 Veil」可编辑；空文档可直接输入；不加载远程图 | 部分自动（smoke 验出窗/标题） | 截图、标题、`config.toml`/`session.txt` |
| A2 | 同上 | 启动即计时，出窗后立刻键入 `hello` | ≤10 秒可输入（正常 <1 秒），无丢字 | 部分自动（smoke 只测 1 秒出窗） | `launch_ms`、计时截图/录屏 |
| A3 | 150%，任务栏常显 | 默认尺寸启动，看四边 | 标题栏、状态栏、滚动条全在工作区，不压任务栏 | 部分自动（几何单测） | 截图、分辨率、缩放、任务栏设置 |

## B 输入保真

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| B1 | 临时 md | `Ctrl+End` 键入 `VEILTYPE`、`Enter`、`Ctrl+S` | 字节 = 原内容 + 标记 | 自动（input 模式字节断言） | 前后 SHA、文档尾字节 |
| B2 | 微软拼音 | 输入 `zhongwen`，看候选窗/组合下划线，空格上屏；再输标点、退格 | 候选窗跟随光标、组合态不丢字、一次上屏、UTF-8 保存 | **必须人工**（SendKeys 被 IME 吞字） | 录屏、输入法版本、上屏文本、SHA |
| B3 | 已输入并保存 | `Ctrl+Z` 保存；再 `Ctrl+Shift+Z` 保存 | 撤销逐字节回上一版；重做复原 | 部分自动（撤销已断言，重做人工） | 两阶段 SHA、截图 |
| B4 | 两段文字 | `Shift+方向`/`Ctrl+A` 后复制粘贴；再从浏览器复制富文本粘贴 | 选区正确；HTML→Markdown 合理；不重复、不丢换行 | 必须人工 | 粘贴前后文本、剪贴板来源 |
| B5 | 20k 段落文档 | 滚到底再回中段连续键入；另跑命令 | 滚动无空白/卡死；击键 p95 ≤16 ms | 部分自动（p95 自动，观感人工） | `perf` 日志、截图、RSS |

```powershell
cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_20k_blocks
$p = "$env:TEMP\veil-20k.md"; 1..20000 | ForEach-Object { "第 $_ 段内容" } | Set-Content -Encoding utf8 $p
pwsh -NoProfile -File tools/walkthrough.ps1 -Exe $exe -Doc $p -Perf -KeepAlive
```

## C 文件

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| C1 | 临时 md | 修改后 `Ctrl+S` | 写回原文件，去脏并显示「已保存 HH:MM」 | 自动（input 模式） | SHA、保存前后截图 |
| C2 | 同上 | `Ctrl+Shift+S` 另存到新目录 | 新文件生成，后续写新路径 | 部分自动（路径/内容需人确认） | 新旧路径与 SHA |
| C3 | >260 字符路径 | `Ctrl+O` 打开、编辑、保存 | 正常打开保存，标题路径可读 | 必须人工 | 完整路径、错误文本 |
| C4 | 只读目录 | `Ctrl+S`，再另存到可写目录 | 报错说明原因，文档仍脏，提示重试/另存；内容不丢 | 部分自动（提示有单测） | 错误框原文、ACL、截图 |
| C5 | 外部改写文档 | 切回 Veil 看提示；分别选「重新加载」「保留当前」 | 重新加载丢弃未保存内容；保留后保存先备份磁盘版本 | 必须人工 | 外部内容、操作前后 SHA |
| C6 | 有未保存修改 | 点 `X`，分别选「保存」「不保存」「取消」 | 三按钮后果与文案一致；取消仍留在脏文档 | 必须人工 | 弹窗截图、关闭后文件状态 |
| C7 | 无参数启动，打开临时文档并修改，停 2 秒 | 任务管理器结束进程；重新双击 exe | 提示「已从崩溃恢复…」，正文完整，`Ctrl+S` 可保存 | 必须人工（强杀不可脚本化） | 强杀前内容、提示截图、`recovery.dat` |

## D 导航

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| D1 | 含重复词文档 | `Ctrl+F`；替换词后 `Enter` 逐个、`Ctrl+Enter` 全部；试非法正则 | 命中高亮跳转；计数正确；非法模式就地报错不崩 | 部分自动（逻辑有单测） | 查询词、替换前后文本、提示截图 |
| D2 | 有最近文件 | `Ctrl+P` 输入前缀，`↑/↓` 选择，`Enter` | 浮层列出候选并可用键盘打开，`Esc` 关闭 | 必须人工 | 候选截图、输入串 |
| D3 | 任意文档 | `Ctrl+,` 改字号/主题，重开 | 即时生效并持久化到 `config.toml` | 必须人工 | 改动项、`config.toml` |
| D4 | 不碰鼠标 | 启动→打开→编辑→保存→查找→关闭 | 全流程可完成，焦点始终可见 | 必须人工 | 中断步骤、焦点截图 |
| D5 | 查找栏/菜单打开 | 按 `Esc` | 浮层关闭，焦点回正文光标，可直接输入 | 必须人工 | 按键后截图、光标状态 |
| D6 | 浅/深主题各一次 | `Tab`/`Shift+Tab` 遍历菜单、按钮、输入框 | 焦点环清晰，两种主题对比度足够 | 必须人工 | 两主题截图、缩放档位 |

## E 窗口

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| E1 | 已有文档 | `Ctrl+Shift+N` 两次，各输不同内容 | 三个独立窗口；关一个不影响其余；关最后一个进程退出 | 部分自动（窗口数可断言） | 各窗口标题/截图、PID |
| E2 | 任意窗口 | 用鼠标拖到最小 | 停在 520×360，不裁切、按钮可点（外部改窗口尺寸不算） | 部分自动（常量单测） | 实际尺寸、截图 |
| E3 | 150%，任务栏常显 | 启动并最大化/还原 | 首窗完整在工作区，不被任务栏压住 | 必须人工 | 截图、任务栏设置 |
| E4 | 依次改缩放 100/125/150/200% | 每档注销重登，重启 Veil，看日志首行 DPI | 分别为 96/120/144/192，比例正确、无模糊裁切、最小尺寸仍成立 | **必须人工**（仅 150% 有自动首帧） | 每档日志、截图、Windows 版本 |

## F 导出

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| F1 | 含表格/代码/图片/Mermaid 的已保存文档 | `Ctrl+E` 导出 HTML，Edge 打开 | 单文件自包含；浅/深主题跟随；Mermaid 内联 SVG | 自动（input 断言 doctype+文本） | 大小/哈希、截图 |
| F2 | 同上 | `Ctrl+Shift+E` 导出 PDF，系统查看器翻页 | 以 `%PDF` 开头；中文/表格/图片/公式/图分页正常；可搜索复制 | 部分自动（触发与哈希自动） | PDF SHA、页数、问题页截图 |
| F3 | 已装打印机 | `Ctrl+Shift+P`，看系统对话框预览，实体打印 1 页 | 预览正确、出纸与 PDF 一致；取消后队列任务消失 | **必须人工**（应用内无独立预览） | 对话框/队列/纸面照片、页数 |
| F4 | 合法 `mermaid` 围栏 | 等渲染，滚过图块，再改源码 | 进程内出图、防抖重渲；无浏览器、无网络 | 部分自动（离线渲染有测试） | 源码、截图、错误提示 |
| F5 | 改为 `flowchart LR` + `A -->` 断行 | 等待重渲 | 提示「图表渲染失败…检查 Mermaid 语法」，源码保留，不联网不崩 | 必须人工 | 非法源码、错误截图 |
| F6 | 同目录放 `veil-logo.png`，加本地图与 `https://` 远程图 | 先保持远程图片关闭，再勾选 | 本地图正常；远程图默认 alt 占位，勾选后才加载；导出永不含远程资源 | 必须人工 | 开关状态、截图、导出 HTML 的 `src` |

## G 信任：零出站验证

关闭浏览器、聊天/同步/更新类程序并记录开始时间。Veil 无遥测、无更新检查、无同步，本项就是验证这一点。**允许**：回环 `127.0.0.1`/`::1`、Windows 自身 DNS/mDNS/LLMNR/DHCP/NCSI/时间同步、其他进程流量；Veil 单实例用命名管道 `\\.\pipe\Veil.SingleInstance.*`，不产生网络包。`Ctrl+单击` 链接交给系统浏览器，须与 Veil 区分；验证时「允许远程图片」必须关闭。可加做**物理断网对照**：拔网线/关 Wi-Fi 后重复启动、键入、Mermaid、导出，功能应不受影响；该对照不能替代抓包归因。

| 编号 | 前置条件 | 操作步骤 | 预期结果 | 自动化状态 | 失败时记录 |
|---|---|---|---|---|---|
| G1 SniffNet | 安装 SniffNet，选活动网卡 | 开始捕获→启动 Veil→键入→滚过 Mermaid→导出；每步记时间点；结束看 Hosts | 无归属 Veil 的新远端主机；新增项须归因到其他 PID | 必须人工（工具不做进程归因） | 主机/端口/时间戳、截图、连接对照 |
| G2 Wireshark | 安装 Wireshark + Npcap | 先录 60 秒基线；清空后录同一流程；套用下方过滤器 | Veil 操作窗口内无新增归属 Veil 的 SYN/远端 UDP；系统噪声应能在基线复现 | 必须人工（结论需判读） | `.pcapng`、过滤器、时间窗 |
| G3 命令行（可选） | 管理员；已装 Npcap | 用 `pktmon` 或 `dumpcap` 录同一流程 | 导出 pcap 用 G2 过滤器复核，结论一致 | 必须人工 | `.etl`/`.pcapng`、命令、网卡号 |
| G4 结论 | 完成 G1–G3 至少一套 | 汇总 | 写明「抓包 X 分钟，veil.exe 远端连接 0 个」；例外列归属 | 必须人工 | 结论与证据索引 |

新建出站 TCP：

```
(tcp.flags.syn == 1 && tcp.flags.ack == 0) && !(ip.addr == 127.0.0.1 || ipv6.addr == ::1)
```

UDP（排除常见系统噪声后与基线对比）：

```
udp && !(ip.addr == 127.0.0.1 || ipv6.addr == ::1) && !(udp.port == 53 || udp.port == 5353 || udp.port == 5355 || udp.port == 1900 || udp.port == 3702)
```

```powershell
# 连接归因：输出为空才说明 Veil 无远端连接
Get-Process veil | ForEach-Object { Get-NetTCPConnection -OwningProcess $_.Id -ErrorAction SilentlyContinue }

# 可选 pktmon（管理员）
pktmon start --capture --pkt-size 0 --comp nics --file "$env:TEMP\veil.etl"
# …执行启动/键入/Mermaid/导出…
pktmon stop; pktmon etl2pcap "$env:TEMP\veil.etl" -o "$env:TEMP\veil.pcap"

# 可选 dumpcap（先用 -D 查网卡号，5 需替换）
dumpcap -D
dumpcap -i 5 -f "not host 127.0.0.1 and not port 53 and not port 5353 and not port 5355" -w "$env:TEMP\veil.pcapng"
```

## H 范围

只验 Windows 11 x64 的单个 `veil.exe`。**Linux 版不在本轮范围**；macOS 同样不提供。平台边界与「Linux CI 仅覆盖 `veil-core`」的口径以 README 的「平台边界」说明为准，本文不重复。相关缺陷另开条目，不计入本轮门禁。

## 30 分钟核心冒烟清单

- [ ] 大小 14,012,928 B，SHA-256 以 `ff23c4d9` 开头、`dd25d0` 结尾
- [ ] 示例页 + `Ctrl+N` 空文档；10 秒内可键入
- [ ] 英文键入→保存→撤销→保存（或 `-EvidenceMode input`）
- [ ] 中文 IME 候选窗、组合态、上屏、UTF-8 保存
- [ ] 查找替换、`Ctrl+P`、`Ctrl+,`、`Esc` 焦点归还
- [ ] 另存为、只读目录报错、关闭未保存三按钮
- [ ] 强杀后不带参数重启，恢复内容完整
- [ ] 多窗口独立；520×360 不裁切；不被任务栏遮挡
- [ ] HTML、PDF 各导出一次，含一个 Mermaid 图
- [ ] Mermaid 错误可操作；远程图片默认关闭
- [ ] 抓包：veil.exe 远端连接 0 个
- [ ] 记录 Windows 版本、DPI、主题、输入法、产物哈希

## 缺陷记录模板

```
缺陷编号：VEIL-MT-YYYYMMDD-NN
标题：
严重度：P0 崩溃/数据丢失 | P1 功能错误 | P2 体验问题
产物：veil.exe 14,012,928 B / SHA-256 ff23c4d9…dd25d0
环境：Windows 11 版本 / DPI / 主题 / 输入法 / 显示器
前置条件：
复现步骤：1. 2. 3.
期望结果：
实际结果：
自动化状态：自动 / 部分自动 / 必须人工
证据：截图、walkthrough.log、导出 SHA、抓包文件与过滤条件
备注：
```

> 事实说明：产品不提供屏幕阅读器（Narrator 等）支持——2026-11-25 决策，见 `.trae/specs/remediate-product-trust-and-accessibility/review.md`。

## I AC9 视觉签名辨识度（盲评）

本节的可自动化部分（滚动条截图、整窗基线）由 `tools/verify/ac9_capture.ps1` 完成；脚本无法自动制造的菜单/模态状态按“盲测方法”第 1 步人工展开后截图，除此之外只有盲评打分需要人手完成。

**前置条件**

- 使用默认构建：

```powershell
cargo build --release --bin veil
$exe = 'target\release\veil.exe'
```

- 本节产物与页首 `release-bundle` 的 14,012,928 B / SHA-256 不同，**不核对页首哈希**；只记录 `(Get-Item $exe).Length` 与构建时间。
- Windows 11 x64，缩放 150%（与 §A/§E 一致），亮色、暗色各一轮；同一轮内 Veil 与参照物窗口像素尺寸一致，以 `ac9_capture.log` 记录的窗口几何为准（建议 1200×800），关闭透明效果与夜间模式。
- 滚动条项须让文档溢出视口（粘贴 200 行以上），否则 thumb 不出现。
- `-Region scrollbar` 是右侧约 20 逻辑像素的固定裁剪；菜单/模态手动截图时按元素外接矩形外扩 24px 裁切。参照工具用同一窗口尺寸、同一 150%、同一裁切尺寸。
- 手动截菜单/模态前，用临时 `VEIL_HOME` 固定主题，不污染用户配置：

```powershell
$env:VEIL_HOME = Join-Path $env:TEMP "veil-ac9-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $env:VEIL_HOME -Force | Out-Null
'mode = "light"' | Set-Content -Encoding utf8 (Join-Path $env:VEIL_HOME 'config.toml')
& $exe
```

- 暗色轮把 `mode = "light"` 改为 `mode = "dark"` 后重启。
- 取证脚本（参数以 `-Help` 为准）支持脚本化合成输入，因此三类截图都可复现。**产品没有 Alt 助记键**（实测 `Alt+F` 会把 `f` 输入正文），菜单弹层只能用合成鼠标点击打开；`Ctrl+,` 打开设置模态（无按钮），未保存确认模态用 `Alt+F4`：

```powershell
$exe = 'target\release\veil.exe'
# 滚动条（直接裁切）
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region scrollbar -Doc .\README.zh-CN.md
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region scrollbar -Doc .\README.zh-CN.md
# 菜单弹层（合成点击展开「文件」，-Region menu 在合成输入下扩展为整窗裁切）
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region menu -ClickMenu -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region menu -ClickMenu -OutDir evidence\verify\ac9
# 未保存确认模态（粘贴一个字符置脏，再 Alt+F4）
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme light -Region modal -SendKeys "^v%{F4}" -OutDir evidence\verify\ac9
pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Exe $exe -Theme dark  -Region modal -SendKeys "^v%{F4}" -OutDir evidence\verify\ac9
```

- 每次合成输入都会在日志/header 中记录（`send_keys` / `click_menu`），便于复跑与审计。
- 未使用合成输入时，`-Region menu` 只裁菜单栏、`-Region modal` 只截整窗基线并 WARN，不能当 `-menu.png` / `-modal.png` 证据。脚本失败时记 `NOT RUN`，不得用旧图、手工拼图或系统截图冒充本节证据。

**三项签名元素与判定**

| 编号 | 元素 | 记 1 分：同时看到 | 记 0 分：任一命中 | 实现依据 |
|---|---|---|---|---|
| I-1 | 滚动条 | thumb 两端为半圆、整体呈胶囊；宽度 `(SCROLL_W - space::XS)`（100% 下 8px，150% 下 12px），圆角等于宽度一半；轨道是与 chrome 区同色的连续色块；鼠标移入只变亮（alpha 170→255），thumb 的宽、长、位置都不变 | thumb 是直角矩形；两端有系统箭头按钮；轨道是系统固定灰；悬停时 thumb 变宽、变长或抖动 | `SCROLL_W = 12`、`space::XS = 4`；绘制圆角 `tw / 2`；轨道 `palette::surface_card`（即 `chrome_bg`）；hover alpha 170/255 |
| I-2 | 菜单分组 | 弹层内所有快捷键提示右对齐成一条竖线；动作名与提示之间固定 24px（150% 下 36px）列距；**全菜单没有一条分隔线**，分组处只有 8px（150% 下 12px）空白 | 出现 1px 横分隔线；快捷键提示左右跳动；动作名被中段省略而提示保留 | 菜单布局 `gap = 24.0 * scale`；分隔项只累加 `space::S`，不画线；悬停底色只覆盖单项，不跨组 |
| I-3 | 模态 vs 普通浮层 | 三个维度同时成立：(a) 同主题下模态底色与浮层不同——亮色模态 `[249,250,252]`、浮层白 `[255,255,255]`，ΔE≈2.7；暗色模态 `[49,49,49]`、浮层 `[45,45,45]`，ΔL≈3.5；(b) 模态阴影更宽更浓（7 层 High 对 5 层 Low）；(c) 150% 下量得模态圆角 12px、浮层 9px（基准 8px 对 6px），误差 ±1px | 两者底色相同；阴影强弱看不出差别；圆角相同或模态更小 | `theme.modal_bg` / `popup_bg`；`HIGH_SHADOW_LAYERS` 7 层 / `SHADOW_LAYERS` 5 层；`DIALOG_RADIUS = 8` / `CORNER_RADIUS = 6` |

逐项勾选（每项只能记 0 或 1）：

- [ ] I-1 看到胶囊 thumb 且不是直角方块/箭头滚动条，记 1 分；否则 0 分
- [ ] I-2 看到右对齐的固定 24px 快捷键列且不是分隔线分组，记 1 分；否则 0 分
- [ ] I-3 看到底色、阴影、圆角三项同时不同且不是任一项相同，记 1 分；否则 0 分

对照：Windows 默认滚动条是直角方块 + 固定轨道色；Windows 原生菜单用分隔线 + 固定 8px 组距。本节要判的是“与系统默认不同且符合上表”，不是“更像某个第三方应用”。

**盲测方法**

1. 六张 Veil 图分三组：滚动条用 `ac9_capture.ps1 -Region scrollbar` 直接截；菜单用 `-Region menu -ClickMenu` 截「文件」弹层（不要用含「关于 Veil」的菜单）；模态用 `-Region modal -SendKeys "^v%{F4}"` 截未保存确认卡片（不要用「关于 Veil」弹窗）。亮/暗各一套，共六张；脚本产出的 `-masked.png`（裁掉顶部 45px 非客户区）可直接用于盲评。脚本不可用时才退回「人工展开 + `Win+Shift+S`」，并在记录中写明。
2. 参照物至少一个同类工具：记事本或 VS Code。滚动条用记事本长文档；菜单用记事本/VS Code 的「文件」弹层；模态用记事本的未保存对话框。参照物与 Veil 同窗口尺寸、同 150%、同裁切尺寸。
3. 遮住标题栏文字、应用图标、任务栏标签，以及任何直接暴露应用名的文字；保留被测控件本体，否则无法量圆角和对齐。
4. 把 Veil 图与参照图随机命名为 A/B/C，交给至少一名未参与开发的人，只给上面的判定表，不给品牌信息。开发者本人不得是唯一评审；若只有本人，结论标注“非盲评”。
5. 揭盲后只对 Veil 那张按 I-1–I-3 打分。参照物只用于回答“是否与系统默认相同”，不参与得分。

**评分与结论**

| 合计 | 判定 | 结论写法 |
|---|---|---|
| 3 | 通过（三项全中） | `AC9 通过（3/3）：I-1/I-2/I-3 全部命中，证据 evidence/verify/ac9/…` |
| 2 | 通过 | `AC9 通过（2/3）：<未命中项> 未命中，证据 <文件名>`；未命中项不得写成“接近通过” |
| 0–1 | 不通过 | `AC9 不通过（<n>/3）：命中 <项>；<项> 判 0，原因是 <子标准>，证据 <文件名>`。必须写清哪一项、哪张图、哪条子标准，不得只写“观感一般” |

**证据命名**

```text
evidence\verify\ac9\<YYYYMMDD>-light-scrollbar.png
evidence\verify\ac9\<YYYYMMDD>-light-menu.png
evidence\verify\ac9\<YYYYMMDD>-light-modal.png
evidence\verify\ac9\<YYYYMMDD>-dark-scrollbar.png
evidence\verify\ac9\<YYYYMMDD>-dark-menu.png
evidence\verify\ac9\<YYYYMMDD>-dark-modal.png
```

亮/暗各一组，缺一张即该项证据不完整。每张图打开确认非空白（至少 1% 非背景像素且控件本体可见）；若截图全黑，先记录文件名与原因，改用 `Win+Shift+S` 或桌面复制（Windows Graphics Capture）重截，不得提交黑图。
