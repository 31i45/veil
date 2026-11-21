# Veil UX 工艺考核报告（spec `polish-veil-ux-craft`）

> **最终结论：`PASS WITH LIMITS`**
> 被测产物：`target/release/veil.exe`，**14,012,928 B = 13.36 MiB**，SHA-256
> **`8DF551F8D363C73DE1F35887042BEC7775E5AB8D1FFEDB50F365AE7E9B456763`**，
> commit **`0d229eb`**；系统 DPI **144（150%）**；`release_ready=true`。
> 证据根：`evidence/ux/{baseline,after,after-v2,…,after-v7,final,…,final-v5}/`。
> **最终证据为 `after-v7/`（10/10 场景 + 文案 6/6 PASS）与 `final-v5/`（全 PASS）**，均绑定上述字节 / SHA / commit。
> `after/` 是暴露 R1–R3 的 v1 中间版；`after-v6/` 与 `final-v4/`（`c19a082`）、
> `after-v5/` 与 `final-v3/`（`ce038eb`）、`after-v4/`（`77b88e2`）、
> `after-v3/` 与 `final-v2/`（`345d8c6`）、`after-v2/` 与 `final/`（`25fea5f3…2b67` / `7c00765`）
> 均为历史中间证据，原样保留，不删除、不改写；
> 本轮所有数字均来自 `after-v7/` 与 `final-v5/`，不沿用旧目录。

---

## 1. 判断标准与本轮边界

判断一个写作表面是否「像一个被认真对待的产品」，只看 `spec.md` 的四件事：

| 标准 | 要回答的问题 | 本轮证据形态 |
|---|---|---|
| **安静** | 首跑是否仍然只是一张纸：没有欢迎弹窗、tour、彩带、示例弹窗 | 首跑截图 + 代码 diff 无新增引导 UI |
| **瞬时** | 反馈是否在用户抬眼处出现，且不会「没有反应」 | 状态栏/通知降级路径 + 生命周期测试 + 窄窗实拍 |
| **诚实** | 失败是否说明原因与下一步；恢复是否说明恢复的是什么、何时、写回哪里 | 文案 + 错误/恢复路径代码与测试 + 实拍 |
| **有分寸** | 破坏性动作是否先解释后果；不可用动作是否解释原因 | 未保存对话框正文、禁用原因绘制、默认焦点 |

本轮**不做**：不新增欢迎页、引导 tour、示例文档弹窗；不新增标签页、插件、云同步；
不改单产物、离线 Mermaid、导出/PDF 架构；不把外部人工项包装成 PASS。

本报告为根目录 tracked 正式版，不沿用草稿占位符或任何未实测的旧数字。

---

## 2. 修改前 / 修改后对照

> 「前」指向 `evidence/ux/baseline/` 的原始截图与事实（`facts.md`），或其记录的
> `5cdf015` 源码行；「后」指向 `evidence/ux/after-v7/` 的最终实拍与 `0d229eb` 源码/测试。
> `after/` 中间版的价值是证明「测试全绿仍可能实拍失败」，其 R1–R3 见 §5。

| # | 场景 | 修改前（baseline） | 修改后（after-v7 / `0d229eb`） | 结论 |
|---|---|---|---|---|
| 1 | **状态栏长路径脏态** | F-c1：`…abcdefghijklmnop.md · 第四层 * · 未保存`，` *` 与 ` · 未保存` 双重脏态同时出现；1350 物理宽下未触发中段省略，窄宽组合未采集 | 脏态只保留 ` · 未保存`；状态栏按「文件名 > 保存态 > 外部变更 > 源码模式」降级（`chrome/mod.rs:552,563,602`），绘制按 Full→NoSource→Core 丢低优先段（`chrome/paint.rs:96`）。700 物理宽 + 205 字符深路径 + 脏态实拍为 `…stuvwxyz.md · 第四层 · 未保存 ｜ 28 字 · 行 4，列 26`，保存态未被通知挤掉（`after-v7/screenshots/f-700-longname-notice/02-*`） | **PASS** |
| 2 | **窄窗口通知** | F0.5：通知只在中段空白绘制，可用宽 < `space::XL*scale` 时**直接不画**；行为上存在「没有反应」 | 通知固定四档落位：中段 → 压缩左文（保存态先预留）→ 占用右文 → 状态栏上方浮层（`chrome/paint.rs:250-272`）；浮层折行、逐行绘制、不省略（`:274-325`）。700 宽失败通知为 6 行完整浮层，无 `…`；900 宽回归无重叠（`after-v7/screenshots/{f-700-longname-notice,b-narrow-900}/02-*`） | **PASS** |
| 3 | **错误通知生命周期** | 所有通知统一 6 s 自动消退（`app.rs@5cdf015:55,1647-1665`），保存失败也会自行消失 | `NoticeKind { Info, Success, Error }`；错误走 `notify_error`，不参与 TTL（`app.rs:1683-1700`）。保存失败、自动保存、快照、导出、打印/PDF 失败均保留；测试 60 s 后仍在、成功提示可消退（`app_tests.rs:17322,17394,17435,17464,17490`）。实拍 Esc 关模态后通知常驻（`after-v7/screenshots/d-save-fail/03-*`）；链接不支持与链接打开失败通知 t=0 与 t=7 s 逐字节一致（`after-v7/screenshots/{l-link-unsupported,l-link-failed}/notice-t{0,7s}.png`） | **PASS** |
| 4 | **导出失败下一步** | 状态栏仅 `导出失败：{}`，且 6 s 消失；模态有原因/下一步但 Esc 后回执无下一步 | 通知改为 `导出失败：{}；下一步：可重试导出，或检查目标目录是否可写后换一个位置`（`i18n.rs:278-279`），走 `notify_error`；模态正文仍为原因 + 下一步（`:283-284`）。测试锁定原因、重试、目标目录（`i18n.rs:897`）与常驻（`app_tests.rs:17464`）；打印/PDF 三类失败文案同口径（`i18n.rs:420-441`；历史实拍 `after-v6/screenshots/print-pdf/tests.txt`） | **PASS** |
| 5 | **崩溃恢复提示** | 旧文案不说明恢复对象、时间与位置；启动路径直接赋值通知 | 恢复提示注入文件名（无路径为「未命名草稿」）、本次恢复时刻与保存去向（`app.rs:893-914`，文案 `i18n.rs:342-346`）。实拍逐字：`已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 20:17。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）`，单行浮层无省略（`after-v7/screenshots/k-recovery-run2/02-*`）；测试 `chrome/tests.rs:1473`、`i18n.rs:1002` | **PASS** |
| 6 | **未保存关闭对话框** | F-e2：正文仅 `未保存的修改在关闭后将无法恢复。`，按钮 `保存 (S)` / `不保存 (C)` / `取消`；默认焦点已是 Primary=保存 | 正文逐项说明三个按钮后果（`i18n.rs:302-303`）；按钮 Primary=保存、Danger=不保存、Secondary=取消（`doc_state.rs:951-958`）；默认焦点仍为保存，Esc 取取消（`view/modal.rs:137-159`）。实拍三行正文完整（`after-v7/screenshots/e-unsaved-close/02-*`）；测试 `modal.rs:1028`、`app_tests.rs:17818` | **PASS** |
| 7 | **菜单快捷键与禁用原因** | 禁用项只静默置灰，无原因；右键与主菜单各自维护同义词条 | 禁用原因单一来源 `disabled_reason`（`chrome/mod.rs:784`），与 `action_enabled` 同源；菜单右侧先显示快捷键，禁用时改为 muted 原因，宽度计入原因（`chrome/mod.rs:343,367-389`），绘制优先保动作名（`chrome/paint.rs:617-632`）。空草稿实拍 `导出 HTML…` / `导出 PDF…` / `打印…` 动作名完整，原因完整 `空文档没有可导出的内容`（`after-v7/screenshots/g-file-menu/02-*`）；右键复用主菜单词条（`view/ctx_menu.rs:124-245`）；测试 `chrome/tests.rs:414,504,1432,1524`、`ctx_menu.rs:878,910,941` | **PASS** |
| 8 | **Esc 焦点归还** | `5cdf015` 无 `focus_return` / `DocFocus`；Esc 只关闭浮层，无「光标/选区/滚动不变」保证与测试 | 正文焦点快照 `DocFocus`，仅在「Esc 关闭了浮层」时归还；鼠标点击、拖选、滚轮后刷新快照（`app.rs:2366-2390`）。测试覆盖 Ctrl+O、查找、替换 Tab、快速打开、设置、模态、焦点环（`app_tests.rs:17650-17838`）；焦点环浅/深实拍可见，深色输入框底 #333A44（`after-v7/screenshots/{h-search-light,i-search-dark}/02-*`）；对比度与 150% 加粗测试 `modal.rs:1164,1194` | **PASS** |
| 9 | **150% 小屏首窗尺寸** | F0.7/F-a3 依据 DPI 未感知的 `1707×912` 推断「默认窗超出工作区」，属口径误记（见 §5 R4） | 首窗按 Windows 工作区 `SPI_GETWORKAREA` 收敛并居中，非客户区与 8 px 边距参与计算（`app.rs:1817-1914,1956-1969`）。真机实测窗口 `593,127–1965,1233`，外框 1372×1106，客户区 1350×1050，完整落在工作区 `0,0,2560,1368` 内，底边 1233 < 1368，任务栏不遮挡（`after-v7/screenshots/j-full-window/window-rect.txt`、`screenshots/j-full-window/02-*`）；小屏收缩由 `app_tests.rs:17533` 与含任务栏的 `17555` 断言 | **PASS** |
| 10 | **最小窗口 520×360** | `5cdf015` 未注册最小尺寸，可被缩到 140×160 并裁掉通知 | 用户拖拽与 config 恢复路径精确钳制到 520×360 逻辑；最小尺寸下保存失败通知首行/末行与状态栏仍完整（`after-v7/screenshots/m-min-size/drag-min/{metrics.txt,notice.png}`）。程序化 `SetWindowPos` 可绕过，裁决见 §5.1 | **PASS（已知边界）** |

---

## 3. U1–U6 逐门判定

> 状态只写 `PASS` / `FAIL` / `NOT RUN`。所有 PASS 均绑定最终 exe 的字节与 SHA，
> 或绑定 `0d229eb` 的代码/测试；外部人工项不在此表冒充 PASS。

| 门 | 判定内容 | 关键证据 | 状态 |
|---|---|---|---|
| **U1 首跑与安静** | 未新增欢迎页 / tour / 示例弹窗；空文档是一张纸；首跑可输入；150% 首窗完整在工作区 | 首跑实拍 `after-v7/screenshots/a-empty-first-run/03-*`：`未命名草稿` ｜ `已新建空白文档` ｜ `0 字 · 行 1，列 1`，无星号；代码 diff 无引导 UI；启动到可见窗口 p50 **593.22 ms** / p95 **602.17 ms**（max 604.06 ms，远低于 10 s），input 模式真实键入→保存→撤销→导出全部断言通过；窗口矩形完整在工作区 | **PASS** |
| **U2 状态栏与通知** | 按优先级降级；900/700 px 无重叠、无静默丢失；通知有降级路径；错误类常驻、成功短暂 | `chrome/mod.rs:552-602`、`chrome/paint.rs:96-158,250-325`；测试 `chrome/tests.rs:1172,1188,1205,1357,1376,1396`；生命周期 `app_tests.rs:17322,17394,17435,17464,17490`；实拍 900/700/保存失败 | **PASS** |
| **U3 错误与恢复** | 所有失败/错误语义文案（A/D 类，穷举 51 个 Key + 锁测试）含原因 + 下一步；纯引导与成功/状态提示（B/C 类）有意不加；导出失败与保存失败同口径；恢复说明对象 / 时间 / 位置；未保存对话框逐按钮后果、默认焦点安全 | 文案质检 6/6 PASS（`after-v7/facts.md` §F2）：链接不支持 / 打开失败、外部文件被删除 / 暂不可读、配置未保存、Mermaid 快照失败逐字含原因与下一步；`all_failure_notices_offer_next_step`（`i18n.rs:1230`）穷举 A/D 类 51 个 Key（含 3 个致命启动、4 个 CLI），断言现状词 + 「下一步」/「重启应用」，并显式排除 B/C 类；链接两条错误通知 t=0 与 t=7 s 逐字节一致（错误类常驻），外部文件两条为 Info 短时提示（6 s）但已含原因与下一步；`i18n.rs:255-364,371-441`；`doc_state.rs:665-670,951-958`；`app.rs:893-914`；测试 `i18n.rs:897,909,984,1002,1037,1077,1162,1199,1230`、`modal.rs:1028`、`app_tests.rs:17818`；实拍 `after-v7/screenshots/{d-save-fail,e-unsaved-close,k-recovery-run2,n-external-*,q-config-not-saved,q-mermaid-snap}` | **PASS** |
| **U4 可发现性** | 可键盘触发动作显示快捷键；禁用项给出原因；同一动作单一主入口 | 命令矩阵 `chrome/tests.rs:414`（覆盖 35 个静态动作、29 个显示快捷键并真实分发）；禁用原因 `:504,1432,1524`；右键一致性 `ctx_menu.rs:878,910,941`；实拍文件/编辑菜单 | **PASS** |
| **U5 键盘与焦点** | 纯键盘完成打开/查找替换/设置/关闭；Esc 后光标、选区、滚动不变；焦点环浅深主题与 150% 可见 | `app.rs:2366-2390`；测试 `app_tests.rs:17650,17665,17727,17763,17790,17818,17838`；焦点环对比与加粗 `modal.rs:1164,1194`；浅/深实拍 `h-*`、`i-*` | **PASS** |
| **U6 工艺与性能不回归** | 150% 八表面每项 ≥4.5；冷启动 p95 ≤1000 ms；20k 击键 p95 ≤16 ms；证据绑定最终 exe | 见下方 U6 数字表；全部证据来自 `evidence/ux/final-v5/`，开始/结束 exe 字节与 SHA 一致 | **PASS** |

**U6 最终数字（绑定 14,012,928 B / `8DF551F8…9B456763` / `0d229eb`）**

| 项 | 实测 | 门槛 | 判定 |
|---|---|---|---|
| 冷启动 `start_to_window_ms`（5 次） | p50 **593.22** / p95 **602.17** / max **604.06** ms | p95 ≤1000 ms | PASS |
| 首帧黑像素（0/30/100/300/800 ms × 5 轮） | 全部 **0%** | <0.1% | PASS |
| 20k 击键（21,000 块 / 300 样本 × 2 轮） | p50 **2.231–2.268** / p95 **2.556–2.615** / max ≤**3.468** ms | p95 ≤16 ms | PASS |
| 150% 八表面视觉 rubric | 最低单项 **4.5**（light/dark PDF 甘特日期字号），其余 5.0 | 任一单项 ≥4.5 | PASS |
| 导出件对照 `task9-final` | PDF light/dark 均 **8120795C…D1BB 字节级一致**；HTML 仅甘特 today 线 2/4 字节（light `x` 21160→21190；dark 21160→21191），其余字节相同 | 无内容回归 | PASS |
| 发布冒烟 `smoke_launch.ps1` | smoke launch PASS（release 门禁） | release 门禁 | PASS |

`release-record.json` 的 build / size_gate / feature_gate / smoke / tests / clippy / checksum / archive
全部 `status=passed`，`release_ready=true`；体积门禁余量 **667,136 B**。
最终提交序列：`55b7984` → `87a7e1d` → `518ae84` → `97d3a60` → `5cdf015` → `eb8825a` → `3d6adf4` → `7c00765` → `345d8c6` → `5b1dbcf` → `77b88e2` → `ce038eb` → `dc6b9b4` → `9759ea3` → `c19a082` → `e044359` → `0d229eb`。

最终串行 workspace 测试 **passed（release 门禁）**：`release-record.json` 的 `tests.status=passed`
（`cargo test --workspace --release -- --test-threads=1`）。附注：可复核分项为 `veil-ui` lib
**1281 passed / 11 ignored**（`perf_20k_blocks` 运行日志共 1,292 项），其余 crate 合计 passed；
本轮未保存逐 crate 的完整总数，故不在此写未实测的汇总值。`evidence/ux/final-v5/report.md`
的 `PASS` 仅指 U6 量化子门，本报告的总判定仍是 `PASS WITH LIMITS`。

---

## 4. 自动验证 vs needs-user

### 4.1 已自动验证（可重复执行）

| 判定面 | 测试入口（`crates/veil-ui/src/`） |
|---|---|
| 状态栏优先级 / 分段 / 700 px 长路径 | `chrome/tests.rs:1172,1188,1205` |
| 通知降级（140 / 320 px）与 700 px 组合 | `chrome/tests.rs:1357,1376,1396` |
| 错误类通知生命周期（60 s / 成功消退） | `app_tests.rs:17322,17394,17435,17464,17490` |
| 导出/未保存/恢复/图片/链接/外部文件/配置/Mermaid 快照文案 | `i18n.rs:897,909,984,1002,1037,1077,1162,1199` |
| 失败/致命文案穷举（A/D 类 51 Key + B/C 类排除） | `i18n.rs:1230` |
| 恢复提示浮层保留对象/时间/去向 | `chrome/tests.rs:1473` |
| 菜单命令矩阵 / 禁用原因 / 右键一致性 | `chrome/tests.rs:414,504,1432,1524`；`view/ctx_menu.rs:878,910,941` |
| 首窗收敛（含任务栏工作区） | `app_tests.rs:17533,17555,17599,17617` |
| 键盘闭环 / Esc 归还 / 焦点环 | `app_tests.rs:17650-17838`；`view/modal.rs:1164,1194` |
| 未保存对话框安全默认焦点 | `view/modal.rs:1028` |

真实窗口自动采集（`tools/walkthrough.ps1`，全隔离 `VEIL_HOME/APPDATA/LOCALAPPDATA/TEMP`）：

- 5 次冷启动 + 逐帧黑像素：`evidence/ux/final-v5/startup/run-01…05/`。
- 真实窗口输入：键入并保存、撤销恢复原文、`Ctrl+E` 导出 HTML，全部断言 PASS。
- 20k 击键两轮：`evidence/ux/final-v5/perf/{run1,run2}.log`。
- 150% 八表面：`evidence/ux/final-v5/visual/{light,dark}/{app,html,pdf}/…`，共 8 个表面逐屏截图与逐项分值。
- 窗口矩形：`after-v7/screenshots/j-full-window/window-rect.txt`（`593,127,1965,1233`，工作区 `0,0,2560,1368`）。

### 4.2 needs-user（NOT RUN，不计 PASS，也不计可控 FAIL）

| 项 | 状态 | 原因 |
|---|---|---|
| 真实 100% DPI 的 light/dark × 编辑态/源码态/HTML/PDF | **NOT RUN** | 需改系统缩放并重启会话；本机只有 150%（144 DPI） |
| 真实 125% / 200% DPI | **NOT RUN** | 同上；未由 150% 结果外推 |
| 真实中文 IME 组字（候选窗 / 组合态） | **NOT RUN** | 自动化只覆盖字母键入与逻辑键 |
| 实体物理打印（纸张、份数、页码范围、取消） | **NOT RUN** | 无打印设备出纸；自动化只覆盖失败/分发路径 |
| 真实 GitHub Actions / Linux 构建与交互桌面 | **NOT RUN** | 本轮未 push，无法实跑远端；不以本地结果替代 |
| 真实断网 / 零后台网络抓包 | **NOT RUN** | 仅代码审计与离线引擎特征门，未做抓包 |

---

## 5. 旧失败 / 残留问题闭环（R1–R3）与基线更正（R4）

`evidence/ux/after/` 保留了 v1 的失败实拍；`after-v7/` 是在最终产物（`0d229eb`）上的同场景复验
（`after-v6/` 与 `final-v4/` 为 `c19a082`、`after-v5/` 与 `final-v3/` 为 `ce038eb`、
`after-v4/` 为 `77b88e2`、`after-v3/` 与 `final-v2/` 为 `345d8c6`、`after-v2/` 为 `7c00765`
的历史复验，结论一致）。这是本轮最重要的质量证据：**单测全绿并不等于用户看得见**。

| 编号 | v1 残留（after/） | 根因 | v7 复验（after-v7 / `0d229eb`） | 判定 |
|---|---|---|---|---|
| **R1** | 700 px 长名 + 脏态 + 错误通知时，` · 未保存` 与右侧统计被通知挤掉 | 通知压缩左文时只留 `identity`，保存态被让位；旧测试分别覆盖长名与短名通知，未组合 | 左文 `…stuvwxyz.md · 第四层 · 未保存` ｜ 右文 `28 字 · 行 4，列 26`；通知为状态栏上方 6 行浮层，全文无省略（`after-v7/screenshots/f-700-longname-notice/02-*`；放大图为历史证据 `after-v2/screenshots/f-700-longname-notice/statusbar-notice-zoom-x2.png`）；修复 `chrome/paint.rs:256-262`，测试 `chrome/tests.rs:1396` | **PASS** |
| **R2** | 文件菜单 `导出 HTML/PDF` 主标签被压成 `导…`，原因被压成 `空文档...出的内容` | 弹窗宽度只量标签 + 快捷键，未计原因；原因固定占 62% 行宽 | `导出 HTML…`、`导出 PDF…`、`打印…` 动作名完整（`…` 为词条自带），禁用原因完整 `空文档没有可导出的内容`；宽度上限 420 逻辑 px（`after-v7/screenshots/g-file-menu/02-*`）；修复 `chrome/mod.rs:343,367-389`、`chrome/paint.rs:617-632`，测试 `1432,1524` | **PASS** |
| **R3** | 崩溃恢复提示中「恢复时间」被中段省略吞掉，只剩对象与去向 | 恢复提示走 `fit_middle`，时间落在被省略的中段 | 单行浮层逐字完整：对象 `recover-me.md`、时间 `20:17`、去向 `写回原文件所在目录「recovery」`（`after-v7/screenshots/k-recovery-run2/02-*`）；修复 `chrome/paint.rs:274-325`，测试 `1473` | **PASS** |
| **R4** | 基线把 `1707×960 / 1707×912` 当作物理屏与工作区，据此断言默认窗超高 | DPI 未感知读数误记为物理值 | 真实物理屏 **2560×1440**、工作区 **2560×1368**、逻辑 1707×960 / 1707×912；窗口 1372×1106 完整落在工作区（`after-v7/screenshots/j-full-window/window-rect.txt`）。更正已写入 `baseline/{README.md,facts.md}` 的 Corrigendum，原始截图/日志与 SHA 不改写 | **已更正** |

未出现用户路径 FAIL。R1–R3 均为**可控失败**，已在最终 exe 上实拍闭环，因此不计入 LIMITS。

### 5.1 最小尺寸裁决（520×360 逻辑）

`0d229eb` 沿用 `MIN_WINDOW_SIZE=(520,360)`、`with_min_inner_size` 与配置恢复路径的 `max()` 钳制
（`app.rs`，测试 `app_tests::min_window_size_keeps_four_notice_lines_and_status_readable`、
`app_tests::initial_window_never_falls_below_min_size_on_tiny_work_area`）。裁决口径：

| 路径 | 实测 | 裁决 |
|---|---|---|
| 用户鼠标拖拽（用户 sizing） | 1372×1106 → 外框 802×596，客户区 780×540 = **520×360 逻辑**，精确钳制 | **PASS** |
| 配置恢复（`config.toml` 480×360） | 外框 802×596 @40,60，客户区 780×540，被最小尺寸钳制；低于解析下限 480×360 的配置整组忽略 → 默认 900×700 | **PASS** |
| 最小尺寸下的错误通知 | 保存失败通知首行与末行、状态栏均完整可见（`after-v7/screenshots/m-min-size/drag-min/notice.png`）；140 逻辑尺寸程序化请求下通知首末行仍可见（`m-min-size/logical-140x160/request.png`） | **PASS** |
| 外部程序 `SetWindowPos` 直接设为 140×160 | 未被钳制（逻辑请求实际客户区 210×240 物理 = 140×160 逻辑；物理请求 140×160 物理） | **已知边界，不判 FAIL** |

`WM_GETMINMAXINFO` 返回 `ptMinTrackSize=802×596` 外框（非零，已注册）。Windows 只在**用户 sizing**
时应用 tracking size；外部进程的 `SetWindowPos` 属 API 层，应用未在 `WM_WINDOWPOSCHANGING` 二次钳制，
因此可绕过。该路径不对应任何 UI 手势或配置输入（配置低于 480×360 会被拒绝），不是用户可达路径；
`after-v7/README.md` 与 `facts.md` 把第 10 项判为 PASS，并把程序化 `SetWindowPos` 记为观察项，
不改变用户路径 PASS 的判定。若被外部强制压到极窄尺寸，通知底部行可能被裁切（见 §6 第 4 条）。

---

## 6. 诚实限制

这些是证据口径或产品行为的真实边界，必须与 PASS 一起阅读；它们不是外部人工项，
也不构成本轮的可控 FAIL。

1. **恢复时间取启动恢复时刻**：快照文件本身没有写入时间戳，提示中的「恢复时间」是本次启动
   执行恢复的本地时刻（`app.rs:913-914`），不是丢失前的最后编辑时间。文案已说明「恢复时间」，
   但用户仍可能按「最后编辑时间」理解。
2. **系统打开/另存对话框用 `FakeDialog` 断言**：`FileDialog` 由真实平台对话框实现，
   自动化用 `FakeDialogs`（`app_tests.rs:400-426`）断言「分发到对话框 / 取消 / 失败出口」，
   不能断言系统对话框内部控件与真实选择结果；后者属 needs-user。
3. **副屏首窗仍按整屏回退**：工作区仅通过 Windows `SPI_GETWORKAREA` 取**主显示器**；
   副屏或非 Windows 无工作区时回退到 winit 显示器整屏矩形（`app.rs:1956-1961,1866-1883`），
   副屏上的首窗可能贴近该屏任务栏。主屏 150% 场景已实拍无遮挡。
4. **极窄客户区浮层的竖直裁切（仅程序化可达）**：通知浮层高度被 `status_y - MENU_H*scale` 夹取，
   绘制时超过状态栏上沿即停止（`chrome/paint.rs:292-313`）。最小尺寸已把用户拖拽与配置恢复
   钳到 520×360（§5.1），但外部程序 `SetWindowPos` 可绕过；当客户区被强制压到约 <150 px
   且通知很长时，底部行可能不可见。140×160 的绘制存在性测试通过（`chrome/tests.rs:1357`），
   但不等同于全文可见。这是 API 层已知边界，不是用户可达 FAIL。
5. **其他非阻断口径**：700 px 下文件名仍按既有策略中段省略（保存态、通知全文、右侧统计不丢）；
   图片类失败提示已在 `345d8c6` 补齐，最终产物 `0d229eb` 保留「原因 + 下一步」
   （读取/保存/格式不支持/文件缺失/远程加载，测试 `i18n.rs:1077`）；链接两条为错误类常驻
   （t0 = t7 s 逐字节一致），外部文件删除 / 暂不可读仍是 Info 短时提示（6 s）但文案已含原因与
   下一步，错过后需从标题/内容自行判断，这一点按短时提示如实记录、不写成常驻；配置未保存与
   Mermaid 快照失败由 `after-v7` 新增实拍验证（`after-v7/facts.md` §F2）；失败文案穷举见
   `i18n.rs:1230`（A/D 类 51 Key）。`PrintWindow` 对空白/低变化帧可能降级为屏幕捕获
   （日志 WARN，属采集启发式）；窗口右下约 8 px 的其他内容来自 DWM 不可见缩放边框，不是裁切。

---

## 7. 红队视角与 Jobs 视角

**红队（先假设 PASS 是假的，再找反例）**

| 攻击 | 结果 |
|---|---|
| 「测试全绿但用户看不见」 | 真实发生过：v1 的 R1–R3 就是单测绿、实拍失败。v7 在最终产物上用同一物理宽度重拍并保留 v1 失败图，未用测试替代观察 |
| 「用均值掩盖低分项」 | 视觉 rubric 采用「任一单项 <4.5 即 FAIL」，最低项 4.5 明示为 PDF 字号，不用均值抵消 |
| 「拿旧 exe 的证据冒充最终」 | 所有最终数字重跑并绑定 14,012,928 B / `8DF551F8…9B456763` / `0d229eb`；`c19a082` 的 `after-v6/`、`final-v4/`，`ce038eb` 的 `after-v5/`、`final-v3/`，`345d8c6` 的 `after-v3/`、`final-v2/`，`77b88e2` 的 `after-v4/`，`25fea5f3…2b67`（`7c00765`）的 `after-v2/`、`final/` 仅作历史对照，`task8a-b`、`task9-final` 只作对照 |
| 「把 needs-user 写成 PASS」 | 六项外部人工项全部保持 NOT RUN，未从 150% 或本地结果外推 |
| 「把 API 绕过写成用户 FAIL」 | 程序化 `SetWindowPos` 可绕过最小尺寸，但用户拖拽与配置恢复精确停在 520×360；按用户路径判 PASS，同时明写为已知边界（§5.1） |
| 「PASS WITH LIMITS 是遮羞布」 | LIMITS 只剩六项外部人工项；可控失败 R1–R3 已实拍闭环，未用限制掩盖 |
| 「报告数字无法复核」 | 每张截图、日志、导出件、`exe-verify.txt`、`window-rect.txt` 都在 `evidence/ux/` 下，含字节数与 SHA-256 |

**Jobs 视角**

这轮没有增加一个功能入口，而是删掉了几类困惑：状态栏不再为了塞通知牺牲「未保存」；
窄窗失败不再「没有反应」；错误类通知不再 6 秒后自己消失（外部文件状态提示仍为短时，
但已带下一步）；恢复提示能回答「恢复的是什么、何时、写回哪」；关闭前知道三个按钮各自会
发生什么；禁用项会说明为什么不能点；Esc 之后光标回到
原来的位置。空文档仍然只是一张纸——没有欢迎页、没有 tour、没有示例弹窗。判断一个写作
表面是否被认真对待，最终看的是这些「用户抬眼处」的细节，而不是功能清单的长度。

---

## 8. 结论

**`PASS WITH LIMITS`**

理由：U1–U6 六门均有最终 exe 上的实测证据或 `0d229eb` 的代码/测试闭环；冷启动、首帧、
20k 击键与 150% 八表面全部达标；R1–R3 可控失败已实拍修复；最小尺寸在用户路径成立
（程序化绕过为已知边界，§5.1）；`release_ready=true`。

**LIMITS（只保留外部人工项，均为 NOT RUN，不是可控 FAIL）**

1. 真实 **100% DPI** 的 light/dark × 编辑态/源码态/HTML/PDF 八表面。
2. 真实 **125% / 200% DPI** 的首帧、几何与视觉。
3. 真实**中文 IME** 组字（候选窗、组合态、上屏）。
4. **实体物理打印**（纸张、份数、页码范围、取消）。
5. 真实 **GitHub Actions / Linux** 构建与交互桌面。
6. 真实**断网 / 零后台网络抓包**。

以上六项需要改系统环境、真实设备或远端 CI 才能验证，本轮不具备对应环境，故不标 PASS；
它们不掩盖任何已发现的代码缺陷，也不构成本轮的可控 FAIL。可控范围内的失败（R1–R3）
已全部闭环，自动门禁与发布记录均为 `passed`。
