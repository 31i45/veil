# Changelog

Veil 版本迭代记录。每个版本的详细修复清单见对应 git tag 的 commit log。

## Unreleased

双发行产物：`veil.exe`（远程 Mermaid）+ `veil-full.exe`（完全离线 Mermaid）。

### Added
- `veil-full.exe`：新增 `crates/veil-mermaid-offline`（rquickjs 0.9 + 完整 `mermaid.js` 11.12.0 + linkedom headless DOM），Mermaid 图表全程本地渲染、零网络；与 `veil.exe` 共用同一 `lib::run()`，只有渲染器实现不同
- 构建：`mermaid-offline` cargo feature + `src/bin/veil-full.rs`（`required-features`）；`release.yml` 以隔离的 `target-default` / `target-full` 同时构建/上传两个 exe 与 SHA-256，并加体积硬门禁（`veil.exe` ≤10 MiB、`veil-full.exe` ≤14 MiB）
- 发布门禁：默认版按 ASCII 内容特征负向检查 QuickJS/linkedom/cytoscape/fcose，完整版正向检查同样的引擎特征（不再检查 `include_str!` 不保留的 `mermaid.min.js` 文件名）；Cargo test/clippy 的非零退出码不再被管道吞掉，Clippy 覆盖 `--all-targets`
- Mermaid 快照：`mermaid_insert_url` 旧键静默忽略，两个发行版都始终写本地 `assets` 相对路径，绝不插入 mermaid.ink URL。源码始终是唯一真值，光标进入显示代码底色/等宽字体的围栏，快照被明确表达为独立且可能过期
- 在线预览提示：`veil.exe` 自动渲染可见 Mermaid 块同样会把源码发给 mermaid.ink，首次自动请求显示一次性第三方提示，不再静默上传；离线实现不提示
- 零级发布门禁：新增 `tools/smoke_launch.ps1`，对两个 release exe 验证「主窗口可见 + 标题正确 + 进程存活 + 无 stderr panic + 正常关闭」；`release.yml` 在两个 exe 构建后执行冒烟，失败即阻断发布

### Changed
- 字体：文本光栅化把行基线（含上下标偏移）吸附到整数物理像素，消除行基线小数带来的垂直亚像素模糊；布局、光标、选区与命中几何不变。拉丁优先字体栈经实际解析与 A/B 目检后否决，维持 `Microsoft YaHei UI` 优先并记录原因
- 导出 PDF 失败语义：保留既有目标文件，显示可操作错误（中文字体/目标路径/权限/文件占用），不再回退打印或位图 PDF；导出为唯一矢量出口
- 文件菜单：导出 HTML、导出 PDF、打印…；「打印…」仍走系统打印对话框 + GDI，并与 PDF 共用 `pdf_layout` 分页
- Mermaid 渲染：`MermaidRenderer::request` / `render_blocking` 改为携带 `(label, source, dark)`：远程实现用 label（mermaid.ink URL），离线实现直接用源码；上层缓存、去重与 pending 匹配逻辑不变
- 结构：`crates/veil/src/main.rs` 拆为 `lib.rs`（`pub fn run()`）+ 两个薄入口，默认产物行为不变

### Removed
- 明确不采用 v1.9.0 的 `veil-mermaid` 实现（Tiny 子集 / 手写 DOM shim / 自有 worker 与缓存），也不提供「从 v1.9.0 取回」的路径
- 应用内「打印预览」：删除菜单项、整页位图覆盖层、预览端口方法与相关文案/测试；预览改为导出 PDF 后用系统查看器查看
- GDI 位图 PDF 回退：删除「Microsoft Print to PDF」位图兜底与「回退打印对话框」路径；导出 PDF 只保留原生矢量管线
- 打字机模式：删除视图菜单开关及相关状态/文案（专注模式保留）；模式切换不再作为可发现设置项

### Fixed
- 启动即崩（0xC0000409）：隐藏首帧路径在 surface 尺寸未设置时调用 `buffer_mut()`，两个 exe 双击无窗口、进程退出。现在任何 `buffer_mut()` 前都先同步 surface 尺寸，隐藏窗口首帧不再依赖首次 `Resized`；补 `PresentSink` 顺序回归测试与真实启动冒烟
- Mermaid 自动预览/导出取源错误：`block range` 含 ``` 围栏行，源码被整段（含 fence）投递，mermaid 报 “No diagram type detected”、mermaid.ink 返回 400；改用 `fenced_code_source` 只取围栏内容，并补回归测试
- mermaid.ink 端点失效：`/img/…?type=svg` 被上游忽略并返回 `image/jpeg`（按 UTF-8 解析必然失败）；改用路径式 `/svg/`，并在 payload 显式下发 `htmlLabels:false`（否则输出 foreignObject，resvg 无法光栅化标签）
- kanban 浅色主题列标题不可读：Mermaid 11.12 默认给 `section-2` 白字 + 浅紫底（对比度不足 2:1）；浅色主题注入深色列标题并加 WCAG AA 回归测试，深色主题保留原浅色标签
- 测试稳定性：默认版远程 SVG mock 使用 20ms 墙钟截止，CPU 争用时会出现假超时；文档记录必须串行执行测试，不改超时或断言口径

### Notes
- 体积（2026-11-20 本机构建，隔离 target）：`veil.exe` 10,062,848 B ≈ 9.60 MiB（较 v1.8.0 的 9,970,176 B +92,672 B，门禁 ≤10 MiB）；`veil-full.exe` 13,992,448 B ≈ 13.34 MiB（离线渲染栈 +3.75 MiB，门禁 ≤14 MiB）
- 测试：串行 `cargo test --workspace --all-features -- --test-threads=1` 1897 passed / 0 failed / 15 ignored（远程 SVG mock 为 20 ms 墙钟截止，并行执行可能假超时）；默认特性 `cargo test -p veil` 215 passed / 1 ignored（差额为 4 项特性门控测试）；`cargo clippy --workspace --all-features --all-targets -- -D warnings` 零警告
- 离线覆盖：flowchart、sequence、class、state、er、journey、gantt、pie、quadrant、requirement、gitgraph、c4context、mindmap、timeline、sankey、xychart、block、packet、kanban、architecture、radar、treemap 共 22 类按内嵌 Mermaid 11.12.0 实际支持逐类验证（含 mindmap canvas2svg 全层级对比度、kanban 列标题与 journey 标签的浅色对比度修复），mindmap/kanban 提供浅深双主题 SVG+PNG 并排证据
- 界面：固定简体中文单语表，不探测系统语言、无运行时切换；正文/导出/图内文字不随 UI 翻译。菜单与快捷键经 195 条命令矩阵走查，缺失项已按证据修复

## v1.9.0 — 2026-11-18

Mermaid L1 内嵌渲染（QuickJS-ng 内嵌官方 Tiny mermaid + resvg 栅格化，路线 A）。

### Added
- Mermaid 图渲染（设置「Mermaid 图渲染」，默认开；关闭后行为与 v1.8 逐字节一致）：预览面板与正文内嵌块直接显示按设备 scale 重栅格化的矢量图——QuickJS-ng（rquickjs）常驻 64MB 栈 worker 线程执行 pin 版本 mermaid 12.3.0 Tiny 子集（官方 ESM chunks 自拼：flowchart/graph/sequence/pie，1,720,018 B 嵌入，无网络依赖），统一 `htmlLabels:false` 产出纯 `<text>` SVG，文本度量由 cosmic-text 注入（CJK 与正文字体栈一致），resvg 全局 fontdb（CJK 兜底 Microsoft YaHei）栅格化为 RGBA；编辑 280ms 防抖、(源码,主题)→SVG 与 (源码,主题,scale)→位图双层缓存，滚动/重绘直绘命中项不重布局
- 安全与降级：3s 看门狗（QuickJS interrupt hook）+ 64MB 内存限制；超时/内存/校验失败转 `MermaidError` 显示并回落源码卡；Tiny 不含的图型（mindmap/architecture/class/state 等）与语法失败走既有 v0 降级卡 + mermaid.live 外链，不显示为错误；HTML 导出内嵌 `sanitize_svg` 清洗后的 SVG；PDF/打印 300dpi 位图嵌入
- 红线更新（v1.9 授权）：exe ≤16 MiB（实测 14,683,136 B ≈ 14.00 MiB，`mermaid-render` 开）、workspace 依赖 ≤240（实测 153）；feature 关闭回落原口径（10,016,256 B ≈ 9.55 MiB / 125 包）

### Notes
- 发布：workspace 版本 1.9.0，tag `v1.9.0`；GitHub Actions `release.yml` 以 `cargo build --release -p veil --features mermaid-render` 构建并上传 `veil.exe`
- spike 实测与结论见 `spikes/quickjs-mermaid-spike/RESULTS.md`（断言三例 PASS、引擎冷启字节码路径 58ms、体积增量 4.93 MiB）
- `mermaid-render` cargo feature 可整体裁掉渲染栈（依赖 125 包、exe 回 ≤10 MiB 口径）

## v1.8.0 — 2026-11-17

组件替换（代码高亮 → `syntect`、配置写回 → `toml_edit`）+ 查找正则模式 + 缓存治理。

### Added
- 查找正则模式：`.*` 按钮或 `Alt+R` 切换（会话内保持）；`^`/`$` 按行锚定，替换支持 `$1` / `${name}` 捕获组；非法模式查询文字转危险色、计数区显示「正则错误」并禁用替换；零宽命中自动忽略；Rust `regex` 线性时间匹配，无回溯爆炸风险
- 图片缓存上限可从 `config.toml` 的 `image_cache_limit_mb` 配置（默认 200，0 = 不限制，合法 0..=4096；旧配置缺键走默认且不提示）

### Changed
- 代码高亮：自研逐行状态机 → `syntect` 默认语法集（75 套）；scope→语义色映射，输出契约 `(start, end, TokenKind)` 不变，复制/打印/PDF/HTML 导出路径零改动。`SyntaxSet` 经 `OnceLock` 惰性加载（首次遇到代码块才加载），逐行缓存 `ParseState`，单字符编辑只重解析受影响行
- 配置：保存时若原文是严格 TOML 且无表段，用 `toml_edit` 原地覆盖已知键，保留注释、未知键与键顺序；非法值、重复键、`[theme]`/其它表段回退整文件重生成。加载仍走既有行解析，`ignored` 计数与夹取语义不变
- 图片缓存：由「首次插入时间逐出」（实为 FIFO）改为真 LRU——命中即刷新最近使用时间，仍在绘制的图片不会被优先逐出
- 外部文件监听：`same_target` 评估后保持「同目录文件名相等」规则，补注释记录不收紧理由（路径形态差异会漏事件）
- 文档：ADR-012 重写为「条件 Go」；spike 文档新增 S1-b 增量复核；README 中英文更新语言覆盖、ADR 索引（新增 ADR-013/014）与全部数字红线

### Notes
- 体积：release exe 9,970,176 B（≈9.51 MiB，较 v1.7.0 本机构建 7,761,408 B +2,208,768 B，余量 515,584 B）；GitHub v1.8.0 发布资产由 CI 构建，为 9,973,248 B。正则模式相对前一构建仅 +36,864 B——`regex` 早已由 `syntect` 的 `fancy-regex` 传递链接，并非早期预估的 +1.2 MB
- 依赖：`veil-ui`+`veil` 外部直接依赖 17（`veil-ui` 8 + `veil` 9）；workspace 传递依赖 125（按 crate 名去重，normal 边，门禁 200）
- 测试：`cargo test --workspace --release` 1797 passed / 0 failed / 14 ignored；clippy `-D warnings` 零警告
- 性能：1000 行代码块单字符编辑 p95 < 0.5 ms（S1-b）；>200 行块首次显示存在一次性 20–100 ms 高亮成本
- 语言覆盖：旧 19 个规范名中 17 个由默认语法集覆盖；`toml` 与 `dockerfile` 回退纯文本；界面文案仍仅中文（i18n 为单语表，无英文条目）

## v1.7.0 — 2026-11-15

「审计闭环 + 本轮能力」：交互缺陷清零、图片/折叠补齐、Mermaid 预留接口。

> v1.3–v1.6 为同分支波次归档（未单独打 tag），其变更随本版本一并发版。

### Added
- 图片三档对齐：独占行 `<img align="left|center|right">`，屏幕按 `width` 偏移、HTML 导出为 `figure.img-center/img-right`、PDF 同步偏移，三端同语义
- 图片点击放大预览：滚轮平移、`Ctrl+滚轮` 或 `+`/`-` 缩放、`0` 适应、方向键/翻页平移、`Esc`/遮罩关闭；`Ctrl+单击` 仍落光标显示源码
- 容器内 `<details>` 折叠：引用/提示/列表项里点击摘要展开/收起，块高与后续块位移随排版更新、不露裸标签；折叠只进视图态（不写文档、不进撤销栈），光标进入正文自动展开
- Mermaid 后端中立接口 `view/mermaid.rs`：`MermaidRenderer`（源码+主题→SVG 或统一矢量场景）、`MermaidScene`、SVG 清理与场景派生；屏幕内嵌块、预览面板、HTML 导出、PDF 共用同一渲染入口；默认未安装渲染器 → 保持源码卡片 + mermaid.live 降级；不引入 wasmtime/JS、零新增依赖
- 代码块语言标签与一键复制（`Ctrl+Shift+C`、右键首项），复制原文不产生撤销/脏标
- 图片 alt 作为题注；折叠 Front Matter 卡片可点击展开；TOC 锚点跳转与指针反馈
- 应用内打印预览（翻页/缩放/`Esc`）与 PDF 链接注解

### Changed
- 表格编辑：点击落点精确到被点中的字形（纯文本格首点即精确），活动格光标随源码偏移逐字移动，不再固定在格首；富文本格仍落格首
- 性能：footnote epoch 与排版键预计算、Front Matter 标志缓存，压低大文档击键成本；同机复测击键 p95 1.297 ms（5,005 块）/ 1.796 ms（10,500 块），保存字节与序列化逐字节一致
- 文档：README 纠正「不做自动配对」的过时描述（v1.5 已交付），补齐本轮能力；补记此前漏写的大纲面板、版本历史、新建窗口、视图缩放/全屏四项已实现能力；语法高亮语言数按实际文法更正为 17 套（19 个规范名）；依赖计数按实测回填（外部直接依赖 11、normal 边传递依赖 101），修正不存在的 `bench/corpus/` 路径说法并补上 ADR-009/010；明确界面文案当前仅中文；明确应用图标与文档图标均为自绘设计，不使用 Typedown 或任何第三方图标；测试与体积基线更新
- 体积：release exe 7,761,408 B（≈7.40 MiB，门禁 10,485,760 B）；GitHub v1.7.0 发布资产由 CI 构建，为 7,763,456 B

### Fixed
- 渲染：Front Matter、嵌套容器引用标记、水平线、行内 details、脚注、硬换行、表格内图片位置、独立图片块高与段距
- 打印/PDF：引用、列表容器内 `<details>` 的闭标签不再作为正文输出（真机抽查发现；回归 `container_details_tags_do_not_leak_into_pdf`）
- 语法/导出：表格切分单一真源、图片题注规则、导出保留代码语言与题注
- TOC：旧式列表、带空格锚点、重复编号
- 审计批次（2026-11-14/15）：解析、渲染、交互、导出、数据安全、平台与安全共 40+ 项（明细见对应 commit log）

### Notes
- `cargo test --workspace` 1755 passed / 0 failed / 14 ignored；clippy `-D warnings` 零警告
- Mermaid 只交付接口与降级：未捆绑任何渲染运行时，不宣称已可渲染

## v1.6.0 — 2026-11-14（波次归档，未打 tag）

### Fixed
- 链接：CJK 自动间距不再在已消隐的 `[` 上生成同起点 fill，修复「只剩左方括号」的半展开
- TOC：生成改无序列表，消除每行字面 `1.`；排版层新增有序列表自动编号（同层连续、嵌套后继续、激活恢复源码）
- 图片：独占一行按视口高度上限、按视觉行判定；支持 HTML `<img width height>` 显式尺寸并在导出保留；缓存键纳入高度上限
- 设置：窗口几何值按可用宽度省略截断；清除按钮带描边与 hover
- 光标：输入/点击/滚轮立即恢复并重置闪烁周期；闪烁到期以 `last_blink` 为基准，消除翻转抖动
- 查找栏：文件/视图类 Ctrl 组合穿透，编辑/剪贴板类仍归查找栏

## v1.5.0 — 2026-11-13（波次归档，未打 tag）

### Added
- `[TOC]`：纯函数 toc 模块生成带 slug 锚点的嵌套有序列表，随编辑自动刷新
- 光标 600 ms 闪烁：IME 预编辑与打印态暂停
- 自动配对：`()` `[]` `{}` `"` `'` 五类，支持选中包裹、成对退格与转义保护，不干预 IME
- 设置模态：10 个配置项，`Ctrl+,` 打开，改动立即生效

### Fixed
- TOC 载入即刷新；设置框聚焦后值文本被焦点环覆盖；设置 Tab 焦点；闪烁到期不重绘；preedit 防护

## v1.4.0 — 2026-11-13（波次归档，未打 tag）

### Added
- 远程图片默认自动加载；「视图 → 阻止远程图片」全局关闭并持久化
- 安全口径：无 Cookie/`Referer`/凭据、UA 固定 `Veil/<version>`、10 s / 20 MiB、重定向 ≤3 且禁 https→http、SSRF 校验；请求去重、失败记忆、代次失效
- 文档改写为零后台网络承诺：除按需抓取远程图片外不发请求，导出/PDF 永不包含远程资源

### Fixed
- HTML `<img>` 远程 `src` 导出修复；details 三角兼容微软雅黑

## v1.3.0 — 2026-11-13（波次归档，未打 tag）

「设计统一」：令牌单源、表面层级、状态过渡与便携文件关联；含 Wave 7 行内 HTML 白名单与 B8/B9 收口。

### Added
- 原生 PDF 排版保真（Wave 4）：按字形宽度换行、H1–H6 分级字号、列表/引用/Admonition/代码块/表格排版、图片与公式嵌入、页边距、页脚 `n / total`、目录页与多级书签；图片/公式失败降级为 alt/源码，无字体时回退 GDI
- 文件关联 `veil.exe /register` / `/unregister`：写当前用户 ProgID、内嵌文档图标（资源 ID 2）、加入「打开方式」并只补 `.md` 空位；`UserChoice` 从不触碰，默认应用仍需在系统设置中选择
- 窗口几何记忆：`config.toml` 新增 `window_width` / `window_height` / `window_x` / `window_y`，启动恢复上次位置与尺寸；越界坐标夹回可见区，最大化不覆盖常规尺寸
- 状态过渡动效：菜单栏、菜单项、大纲、快速打开、右键菜单、查找栏与对话框按钮 120 ms ease-out cubic 淡入淡出；跟随系统「显示动画」，空闲路径零定时器
- 设计令牌：`tokens::palette` / `state` / `space` / `stroke` / `motion`；`Theme::surface(SurfaceLevel)` 表达 Base / Card / Popup 三层
- 历史、单实例多窗口与 Tab 焦点、i18n、外部变更四按钮模态、启动读取失败应用内错误模态
- Wave 7 顶流行内 HTML 白名单：kbd/sub/sup/mark/abbr/code/a（成对）+ br/img（自闭合）+ details/summary（块级折叠）；预览/打印/PDF/HTML 导出语义一致，导出剥 script/style、白名单外 escape
- 空 Mermaid 块「点击插入模板」：左键命中后弹出三条模板（流程图/时序图/类图），插入 body 起点并把光标定位到插入内容之后，单次 undo 可整体回退

### Changed
- 图标体系改为图片真源：应用图标重做为「白页 + 面纱垂坠 V」；文档图标以自绘设计稿 `icons/Markdown.png` 为准（≥32px 完整保留折页 M↓，≤24px 同色系简化徽标）；两套图标均为 Veil 自绘，不使用 Typedown 或任何第三方图标资源。各 7 档（16/20/24/32/48/64/256）由 `build.rs` 以 `include_bytes!` 嵌入，`icon.rs` 只保留窗口图标 RGBA
- 浮层收敛为单线：只保留边界线，分组改用间距；查找浮栏改版（按钮与计数重排、无命中空态、滚动条命中刻度）
- 字体显式指定：UI 雅黑 13 px、正文 16 px、代码 Consolas 14 px；README 徽章改为 CI 状态 + 动态 Release
- 体积门禁临时统一为单一 release exe ≤ 10 MiB；当前值与基线口径以 README「体积门禁」表为唯一真源

### Fixed
- 代码块与行内代码中文不再空心/描边：等宽路径显式 Consolas，Han 回退 `Microsoft YaHei UI`，与正文同族
- 行内 HTML 装饰（kbd/mark/abbr/sub-sup/code/pre）按下标偏移正确绘制并覆盖表格单元格；abbr 点线 + title tooltip；表格内 `<br>` 换行；块级 details 展开正文可见、折叠行高不裁切
- 行内代码中文不再多出反引号；中置 Front Matter 不再丢行
- 导出：编号标题不再被当有序列表、行内 `<a href>` 不再丢失、交叉标签不再畸形，补齐 kbd/mark/abbr/details/summary/img 的导出 CSS

### Notes
- 历史体积口径（Phase A 基线，仅存档）：运行时代码（非图标）6,048,424 B（门禁 ≤ 6 MiB）、合计 exe 6,105,088 B；当时两套图标合计 56,664 B，不计入代码门禁
- Wave 7 落地时 release exe 7,031,808 B；真机走查修复后 7,056,896 B（门禁 10 MiB 内）
- 真机走查（2026-11-10）：主界面、查找栏、文件菜单（含最近文件）、未保存确认模态、视图菜单、大纲面板渲染正常；动效时序由单元测试覆盖，未做逐帧截图取证
- workspace 传递依赖 117（门禁 ≤ 200）

### 战略级推翻（2026-11-12）
- 推翻 D10⑤「HTML 白名单不做」裁决，改立项 Wave 7 N10-C 顶流行内 HTML 白名单（零脚本、零外链、纯行内 + 块级折叠）。根因：用户定调所有细节必须完美，要求对齐 Typedown / Obsidian / Notion 行为
- 处理规范：**不得静默改写已勾选项**，必须新建推翻 SubTask 与主台账 D10 子条目，保留历史 + 明确新范围 + 交叉引用
- 实现入口：`.trae/specs/extend-n10-top-tier-html/spec.md`

## v1.2.0

「手感与表达」：行级操作、可读行宽、状态可见性与焦点表达。

### Added
- 行移动 `Alt+↑` / `Alt+↓`：移动当前行，光标随行，一次 undo 单元
- 翻页 `PgUp` / `PgDn`：按视口翻页并保留目标列
- 按词删除 `Ctrl+Backspace` / `Ctrl+Delete`：复用词边界，不跨行
- 查找「区分大小写」开关（默认关、会话内保持）；`Enter` 替换当前命中并前进，`Ctrl+Enter` 全部替换
- 可读列宽：窗口宽度 >1600 px 时正文列约 40 个全角字宽并居中（≤1600 px 行为不变）
- H4–H6 分级：标记区按 2/4/6 个空格做层级缩进
- 状态栏「已保存 · HH:MM」/「未保存」：保存失败保留脏标记并可见提示
- CLI 打开失败：stderr + 可见错误框（`MessageBoxW`），以退出码 1 结束
- 光标表达焦点：聚焦实心、失焦空心，零定时器

### Changed
- 语法注释与 grid/装饰线对比度达标：注释 明 5.04:1 / 暗 4.98:1；grid 明 3.34:1 / 暗 3.55:1
- 标题段距 16 px（正文 10 px），标题与正文的层次更明显
- 菜单/导出项省略号：`导出 HTML` / `导出 PDF` 去掉多余 `…`，仅需后续对话框的项保留

### Fixed
- `Alt+↓` 行移动后画面滞后：行移动不改变块 id 与行区间，布局缓存键不含内容指纹；改为行移动后递增会话代次（回归用例 `line_move_refreshes_layout_without_further_edit`）

### Notes
- 版本号 1.2.0（workspace 版本与 exe FileVersion/ProductVersion 一致）
- release exe 6,043,136 B（门禁 ≤ 6,291,456 B，余量 248,320 B）
- `cargo test --workspace` 1132 passed / 0 failed / 13 ignored；clippy 零警告
- 击键 p95 6.781 ms（5,005 块）/ 12.379 ms（10,500 块），基线 6.677 / 12.488
- 冷启动（进程内首次编辑）290.0 ms，预热后 130.1 ms；本机实测，受系统文件缓存影响
- Windows 10 未做真机记录，不宣称已验证；Mica 不可用时降级为纯色

## v1.1.0

未单独发布，其变更并入 v1.2.0。

### Added
- 菜单键盘导航：主菜单与右键菜单支持 ↑/↓ 选择、Enter 执行、Esc 关闭，边界钳位，复用既有悬停高亮
- 表格行列增删与列对齐：右键 6 项结构操作（上方/下方插入行、插入左列/右列、删除行、删除列；表头无行操作、单列无删除列），左/居中/右对齐只改写分隔行，一次 undo
- 单元格富文本：非活动单元格渲染粗体/斜体/删除线/行内代码/链接色并隐藏标记，活动单元格显示 Markdown 源码；窄列仍走禁则折行
  - 降级说明：极少数单元格（剥标记结果与内核 `cell.text` 不一致，如含转义 `\|`）该格降级为纯文本显示，其余单元格均为富文本；折行策略未降级
- 源码模式 `Ctrl+/`：全文显示 Markdown 源码（含表格、公式块），切换保持光标与滚动位置；打印/导出始终为渲染态
- 直接导出 PDF `Ctrl+Shift+E` / 文件菜单「导出 PDF…」：默认输出到文档同目录 `<stem>.pdf`（重名递增 ` (2)`/` (3)`），支持目录页、页码、页边距（默认 20 mm），配置写入 `config.toml`；依赖系统「Microsoft Print to PDF」，未保存草稿或无 PDF 打印机时回退系统打印对话框并提示
- 性能探针：`VEIL_PERF_PROBE=1` 输出 `[perf] {json}`（key_us/frame_us/rss_mb/dirty_blocks/total_blocks/rescan_lines/undo_depth）；`tools/walkthrough.ps1 -Perf` 汇总 p50/p95/p99、RSS、rescan 比例
- HTML 导出主题化：内联 CSS 跟随当前浅/深主题（背景、正文、弱化色、链接色、代码底色、边框），深色页面含 `color-scheme: dark`；仍为单文件自包含、无外部资源
- 链接用系统默认程序打开：`Ctrl+单击`（鼠标手势，非新快捷键）http/https/mailto 链接交给系统默认浏览器/邮件客户端，光标不移动；链接右键菜单新增「在浏览器中打开」（位于「复制链接地址」之前）；非白名单协议只提示不打开，Veil 自身不发起网络请求
- GFM 脚注：`[^id]` 引用与 `[^id]: 内容` 定义；渲染态引用显示链接色 `[n]`，定义块隐藏标记并显示「编号 + 内容 + ↩」，编号按全文首次引用顺序；`Ctrl+单击` 引用跳到定义、`Ctrl+单击 ↩` 跳回首次引用；源码模式显示原始标记；HTML 导出生成 `<sup>` 引用与含返回链接的文末脚注列表
- 文档内锚点跳转：`[文本](#slug)` 的 `Ctrl+单击` 跳到对应标题；导出 HTML 的标题带 `id="<slug>"`；重复标题按 `名称`、`名称-1` 去重（与导出共用 `heading_slug` / `heading_slugs` 规则）
- 菜单分组分隔线：文件/编辑/视图三个主菜单按语义分组（新建打开 / 保存 / 导出打印 / 退出；撤销 / 剪贴板 / 格式 / 查找 / 插入），右键菜单同样分组（含表格「行 / 列 / 对齐 / 剪贴板」），最近文件块上下各一条
- 菜单项禁用态：无可撤销步时「撤销」禁点、无重做步时「重做」禁点、干净且有路径时「保存」禁点；禁用项灰显、不参与悬停与键盘导航，点击后菜单保持打开
- 草稿态导出：无关联文件时「导出 HTML / 导出 PDF」先弹保存对话框选定输出位置（取消即中止、不产生文件）；无路径草稿按 `Ctrl+S` 直接转为「另存为」
- 更一致的路径与标题：窗口标题显示文件名（不再显示完整路径）、状态栏显示 `文件名 · 父目录`、保存/另存为/恢复/导出提示统一为「动作：路径」并共用同一路径标签

### Changed
- 快捷键：`Ctrl+P` 改为快速打开（与 VS Code 一致），打印移到 `Ctrl+Shift+P`；查找栏内 `Ctrl+P` 可穿透打开浮层、`Ctrl+Shift+P` 不穿透
- 菜单文案：导出项去掉多余的省略号（`导出 HTML` / `导出 PDF`），仅真正需要后续对话框的项保留 `…`
- 链接改用主题链接色（浅 `#2B6CB0` / 深 `#7FB4F5`），不再与正文同色
- `LayoutEngine::layout_block` 增加 `reveal_all`，源码模式复用同一排版路径
- `Editor::append_table_row` 更名 `replace_table_block`，表格结构变更统一走整块替换
- 发布门禁：release exe 体积断言改为 4–6 MiB；版本断言改为 `^1\.`
- 内部端口变更：`Exporter::export_html` 增加 `ExportStyle` 与 `target` 参数（`veil` 产品层同步），调用方在主题切换点传入当前主题快照；`FileDialog` 新增带默认实现的 `save_export`（导出目标对话框）。对外 CLI / 配置不变
- 本批无新增依赖，`Cargo.toml` 未新增 crate

### Fixed
- `table_delete_row` 在仅两行正文时误删两行
- 链接与正文同色，渲染态无法辨识

### Removed
- `Style.underline`（从未渲染的死状态）

## v1.0.0 — 2026-11-05

首个稳定发布版本：单机单用户纯离线 Markdown 编辑器，零 webview、零外部运行时。

### 内核与文档模型
- 三 crate 单向依赖架构：`veil-core`（纯文本 + 增量解析）→ `veil-ui`（winit + softbuffer 自绘）→ `veil`（平台接缝）
- 增量语义树解析：脏块局部重同步，击键即重排
- 撤销/重做：事务模型，"连续性合并 + 光标移动/扩选断组"（VS Code 式）

### 编辑
- grapheme 边界光标导航（CJK 安全）
- 选区输入替换、撤销单步
- Ctrl+B 粗体 / Ctrl+I 斜体 / Ctrl+K 链接，结构感知 toggle（嵌套 `***` 正确解包裹）
- Ctrl+Shift+I 图片插入（独占段）

### 排版与中日韩细节
- 中英自动间距
- 行首/行尾禁则（正文段落 + 表格单元格）
- 满行悬挂标点
- 表格单元格内嵌网格态（Tab/Shift+Tab 导航，末格 Tab 追加行）
- 公式（latex-rust 纯 Rust 渲染）、图片相对路径、源码标记 reveal 展开/收起
- 超长单行降级提示
- 代码块语法高亮：16 种语言的关键字/字符串/注释/数字/类型着色（自研逐行状态机，不引入 syntect）

### 交互与视图
- Ctrl+滚轮缩放 / Ctrl+=Ctrl+- 档位 / Ctrl+0 还原
- F11 无边框全屏，Esc 只关浮层不退出全屏
- 专注模式 / 打字机模式（互斥切换）
- Ctrl+P 系统打印对话框（自带预览页 + Microsoft Print to PDF）
- Ctrl+Shift+P 快速打开（最近文件 + 当前目录候选）

### 写作闭环
- 中文口径字数统计（CJK 逐字 + ASCII 词）
- 任务列表复选框点击切换
- HTML 导出

### 配置与主题
- 浅色 / 深色 / 跟随系统即时切换 + 重启保持
- 深色标题栏 / Mica（Win11）
- 配置持久化到 `config.toml`

### 数据安全
- 任务管理器杀进程后自动从 recovery.dat 恢复草稿
- 外部修改 1 秒内提示 + 覆盖前备份
- 外部删除自动重建
- 无路径草稿 Ctrl+S 弹另存为，不静默丢弃
- 关窗 / Ctrl+N / Ctrl+O 未保存三选一 MessageBox

### 基线数字
- release exe：5.86 MiB（无外部 DLL 依赖）
- 冷启动：145–200 ms
- 击键 p95：7.12 ms
- 10 万字 150% DPI 滚动峰值：28.83 MB
- cargo test --workspace：910 passed / 0 failed / 12 ignored
- cargo clippy：零警告
