# Veil 架构审计（六设计原则）

> 只读审计，2026-11-05。证据格式 `文件:行`（路径相对仓库根）或 ADR 编号。未修改任何代码、未重跑测试与基准，红线数值引用 README 实测口径（README.md:52,54-70）。审计对象：workspace 3 crate、`veil-core`（buffer/marks/history/syntax/session）、`veil-ui`（document/view/chrome 三域 + app/editor/interaction/paint/mode）、`veil`（main + platform 端口实现）。

## 0. 结构现状（先给结论所需的最小事实）

- 三 crate 单向依赖：`veil` → `veil-ui` → `veil-core`；`veil-core` 依赖仅 `lapce-xi-rope`，无 GUI/异步（crates/veil-core/Cargo.toml）。
- `veil-ui` 非路径直接依赖 3 个（winit/softbuffer/cosmic-text）+ `veil-core`；`veil` 5 个（arboard/latex-rust/png/pulldown-cmark/notify）；合计 8，传递 141（README.md:69-70）。
- 状态模型按 ADR-009 五分类落地，`App` 顶层收敛为 10 个子结构（app.rs:336-352），`Revs` 代次 + 因果键 memo 取代手工失效（view/cache.rs:1-16，ADR-009 决策四）。
- 本轮会话最关心的三条路径均已单源化：按键路由（mode.rs:57-60 + 122/135）、禁则表（view/linebreak.rs:15-26）、表格识别（syntax/block.rs:1394 `table_rows` / 1507 `parse_table_cells`）。

---

## 1. 第一性原理（真值先于表现，派生必须可重算）

### 做得好的

- Markdown 字节是唯一真值：`Session::open` 只经 rope 存字节，`serialize` 原样返回（session.rs:39-47, 155-161）；编辑一律经 `Session::apply` 返回脏块集合，渲染层不认识语法（session.rs:26-30）。
- 派生由真值唯一决定且可丢弃：索引/布局/光栅/公式全部落在 `Cache`，键失配即重算，`Cache::prune` 不承担正确性（view/cache.rs:6-16，ADR-009 决策四）。几何索引 `BlockIndex` 明确为“全量块枚举的唯一缓存来源”（app.rs:341-342，block_index.rs:2-7）。
- 文档命令与视图零耦合：`veil-ui` 不知 `veil-core` 的 `Block` 私有类型，只消费 `BlockId/BlockKind/Range`（veil-core/Session 面）。

### 可改进的

- 【中】`Session::blocks()` 每次返回全量 `Vec`（session.rs:155-161）并在 `refresh_index` 每次编辑时分配一次；对 1 万块文档即每击键一次 O(块数) 堆分配。红线仍达标（p95 7.1ms），非阻塞，但接口形状迫使调用方复制真值快照。
- 【中】禁则真值是单源（view/linebreak.rs:15-26），但**成形策略有两套**：正文用 WJ 胶水 + `Wrap::WordOrGlyph`（view/layout.rs:928 注入胶水、542-544 设折行策略、`build_typo_subs` 924-950），表格单元格自写贪心折行（view/table.rs:218-285）。两套策略对“无可断点”的处理不同：正文允许悬挂（`hang_shift`，view/layout.rs:832），表格直接溢出（view/table.rs:259-273），窄列标点串会被列宽裁剪。

### 反模式

- 【低】`veil-core` 头注宣称“唯一公开入口是 Session 门面”（lib.rs:3-4），实际还导出 `emphasis_spans/list_signature/task_span`（lib.rs:14-16；定义见 syntax/mod.rs:497 与 17）。其中 `list_signature` 无产品调用方，只服务于测试夹具。公共面与“按实际使用定义导出”的承诺（ADR-009 S5）漂移。

---

## 2. UNIX 哲学（一个模块一件事；机制与策略分离）

### 做得好的

- 平台能力全部经 trait 端口注入（app.rs:46-146 定义 8 个 trait，153-168 打包 PersistEnv/Doc，AppPorts 356-372），实现留在 `veil`（main.rs 各 `impl Clipboard/FileStore/...`，platform/mod.rs:1-241）；`veil-ui` 不知道 winit 之外的平台细节，符合 ADR-004。
- 按键策略是纯函数、无副作用、可穷举测试（mode.rs:57-60）；IME 落点判定与按键路由同处一份（mode.rs:21-29, 135）。
- chrome 自绘、打印页帧复用同一排版引擎（README.md:31-40，ADR-005/ADR-011），不为两种呈现建两套几何。
- 导出器以 `Exporter` 端口外置（veil/src/export.rs 提供实现，veil-ui 只持有 trait），方向正确。

### 可改进的

- 【中】绘制模块承载编辑策略：表格键盘行为散落在 paint.rs——`tab_step`（321-375）、`table_shows_grid`（426 附近）、`table_ctrl_enter`（1579-1601）。这些是文档/交互域的决策，paint.rs 应只消费状态。它们同时是下文【高】1 的温床（该处直接拿 `session_mut`）。
- 【中】“提示 + 日志 + 重绘”未收敛为单一出口：`chrome.notice` 在 doc_state.rs 中有 20 处直接赋值（31、85、105、133、140、218、270、282、295、315、319、338、352、357、375、392、433、442、475、512），失败分支普遍手写 `notice + eprintln + request_redraw` 三连。实例：`maybe_recovery_write` 失败分支（218）只设 `notice`，其后未见 `request_redraw`，提示可能不显示直到下一次输入。
- 【中】两条“文件不存在”策略并存：`unique_asset_name` 用 `dir.join(..).exists()`（main.rs:294 起），冲突副本用 `store.load(&cand).is_err()` 当“不存在”（doc_state.rs:521-531）。后者把权限/IO 错误误判为可写入的空位。

### 反模式

- 【低】模块命名与职责错位：`document/session.rs`（最近文件/会话恢复）与 `veil-core/src/session.rs`（文档门面）同名不同义；`veil-ui/src/html.rs` 实为 HTML→Markdown 粘贴转换（html.rs:1-15），而 Markdown→HTML 导出在 `veil/src/export.rs`。

---

## 3. 二八定律（20% 的投入覆盖 80% 价值）

### 做得好的

- 无 GPU、无插件、无网络、无自研排版引擎；表格只做网格假说 + 源码态回退（ADR-011）。
- 只保留 15 个端口槽位（app.rs:356-372）对应 15 个 trait，不为假想平台预付抽象；`Print/NoPrint` 是唯一“成本 vs 保真”显式取舍点。
- 测试预算集中在行为：911 项全绿、`veil-core` 209 项、41 条 e2e（README.md:52,65-66；2026-11-05 复测，923 项用例含 12 ignored）。

### 可改进的

- 【中】为状态栏字数付 O(n) 全量拷贝：`refresh_stats` 取 `text_slice(0..len)` 生成整篇 `String`（stats.rs:46-50），而它挂在 `refresh_index`——编辑、撤销、打开、新建、resize、主题切换都会走。10 万字即每次击键一次全量字符串化；红线未破，但这是典型“低频价值、高频成本”。
- 【中】`Editor::full_text()` 在 Ctrl+B/Ctrl+K/Ctrl+Shift+K 等局部操作里整篇克隆（editor.rs:197-201，调用点 178/674/717/781/833）。一次按键的 20% 局部信息需求换全量 rope→String。
- 【中】表格每单元格两趟 shaping（本征宽 + 折行宽）再逐格光栅（table.rs:98-156, 172-213）。缓存键正确，但首屏与缩放时是重投入；v1 表格规模小，可接受。

### 反模式

- 【低】paint.rs:1625-1794 的表格变换区有 8 个 `#[allow(dead_code)]` 标记（1626、1640、1646、1656、1673、1689、1710、1740），其中 `table_delete_row`（1690）、`table_insert_column`（1711）、`table_delete_column`（1741）、`parse_aligns`（1657）无任何调用点，仅 `table_append_row`（1674）被 Ctrl+Enter 使用；这是为 T3 预支的整套代码，且与“只实现了追加”的产品能力不对称。

---

## 4. 极简主义（删到不能再删）

### 做得好的

- 3 crate、单一运行时配置键 `mode`（README.md:26）；无 feature 开关矩阵、无 workspace 级构建脚本。
- 端口聚合为单结构，避免把 15 个 `Box<dyn>` 摊到每个方法签名（app.rs:354-372）。
- 撤销历史、Marks、IME 均为自研最小实现，未引入概念更重的第三方层（history.rs / buffer.rs 头注）。

### 可改进的

- 【中】两个“打开新文档”入口各写一段约 28 行同构重置：`open_path_now` 的重置段（doc_state.rs:452-479）与 `new_draft`（doc_state.rs:492-517）——persist 复位、代次递增、滚动、goal_x、大纲滚动/悬停、搜索、索引、大纲刷新、三类可见性、几何逐项重复；仅 `note_opened` 一处不同。下次新增重置项必漏其一。
- 【中】`veil-ui/lib.rs` 为 bench example 导出内部类型（`BlockLayout/Frame/LayoutEngine/BASE_FONT_SIZE/BASE_LINE_HEIGHT`，lib.rs:48-49），公共导出三十余项；“以产品实际使用定义导出”（ADR-009 S5）被测试便利性侵蚀。
- 【低】注释漂移：app.rs:346-348 称“11 个外部注入的端口”，同文件 354 行称“15 个平台端口”，结构实为 15 字段（356-372）；app.rs:332 的 Input 清单漏列 `user_zoom/zoom_accum/drop_file`。
- 【低】派生值寄居输入态：`theme: Theme`（派生调色板）与 `system_dark`（真输入）同放 `InputState`（app.rs:245-246）。虽有 `set_theme_mode/refresh_theme_visuals` 统一处理（app.rs:229-230），仍与 ADR-009 决策一的分类表不符。

### 反模式

- 【低】`App` 全字段 `pub(crate)`、跨域直写（app.rs:326-328 自述“不经访问器转发”）。约束只靠文档与测试，信息隐藏被主动放弃——这正是【高】1 能以“一行 `session_mut`”绕过的结构性前提。

---

## 5. 代码整洁之道（函数组织、命名、错误分层）

### 做得好的

- 注释解释“为什么”而非“是什么”：Marks 偏置与坍缩语义（buffer.rs:101-105）、列表块扫描为何按行而非按项（syntax/block.rs 模块头注）、重排为何延后到帧（app.rs:302-304）。
- 核心错误类型化（`EditError`）与 UI 边界 `String` 分层明确（app.rs 端口 trait 签名）；`clippy -D warnings` 零告警（README.md:52）。
- 文件名即领域名，`view/{layout,linebreak,table,highlight,cache}.rs` 切分与职责基本一一对应。

### 可改进的

- 【中】超长函数密集：`paint.rs::render` 439-730（292 行，集索引兜底、尺寸兜底、懒排版、滚动钳位、逐块绘制、选区/删除线、公式/表格/图片、chrome、缓存回收于一身，722 处还调 `prune_viewport`）；`paint_search` 895-1157（263 行）；`layout.rs::layout_block` 446-632（187 行，含 cap、占位、样式、高度、图片定位）；`editor.rs::apply_inner` 340-511（172 行）与 `apply_wrap` 665-776（112 行）。
- 【中】错误处理分层只有“类型 vs String”两级，缺少“呈现层”单点：`chrome.notice` 赋值散落各域（见 2 节），同一失败语义在不同路径下可能只打印不提示、或提示不重绘。
- 【低】`app_tests.rs` 单文件 6856 行（全仓最大文件），其中 11 个以上按场景定制的 `Ports` 字面量（335、448、658、841、939、1019、1430、1470、1816、2364、2473…）各自重复装配 15 个哑端口，新增端口需逐一改。
- 【低】`doc_state.rs` 不含 `DocState`（结构定义在 app.rs:276-283），文件按动作命名而非按结构命名，读者预期落空。

### 反模式

- 【低】`notice + eprintln + request_redraw` 手写三连，无 `fn fail(&mut self, msg)` 收口；这是“重复入口”的呈现层版本。

---

## 6. 软件设计哲学（深模块、信息隐藏、错误即异常、复杂度守恒）

### 做得好的

- `Session` 是深模块：一个门面隐藏 rope、Marks、历史、语法缓存四层（session.rs:1-30），调用方复杂度被吸收。
- 因果键 memo（`Revs { theme, session, image_base }` + 宽高比/块 id）把“何时失效”从 30 处手工调用归零（ADR-009 决策四，view/cache.rs:1-16）。
- 复杂度守恒处理得当：表格不引入排版引擎，而是“网格假说 + 源码态回退”；导出复用端口而非第二条渲染链（ADR-011）。

### 【高】1 — 表格 Ctrl+Enter 绕过 `Editor` 门面，脏标不置位，存在内容静默丢失路径

- 证据：`paint.rs:1579-1601` 的 `table_ctrl_enter` 先用纯函数 `table_append_row`（1590）算出新源，再直接 `self.editor.session_mut().apply(Edit::Replace{..})`（1595）并只调 `after_edit`，未走 `Editor::apply`。`dirty` 仅在内容编辑路径置位（字段 editor.rs:61，判定 87-89，赋值 272/292/305/335，分别对应 apply/cut 路径/replace_range/replace_all）；`set_dirty`（93-95）明言“正常编辑路径不得调用”；`session_mut` 是 `pub` 逃逸口（106）。
- 后果链（均为真值驱动，非风格问题）：
  1. 自动保存永不到期：`autosave_due` 要求 `editor.is_dirty()`（doc_state.rs:160-171）；崩溃恢复写同样（185-196）。
  2. 关窗不提示：`resolve_unsaved` 对非脏文档直接返回 true（doc_state.rs:368-371），`confirm_close` 随即放行（401-407）→ 追加行后关闭或 Alt-F4，内容丢失且无提示。
  3. 标题与状态栏不翻转：`after_edit` 末尾 `update_title` 读 `is_dirty`（interaction.rs:59-60）。
  4. 光标坍缩：追加行是对整块 `Replace`，caret 落在替换区间内部，`transform_marks` 将其坍缩到区间起点（buffer.rs:106-128）→ 加行后光标跳到表首，且 `after_edit` 的 `scroll_to_caret`（interaction.rs:59）把视口带过去。
  5. 无回归测试：全仓仅此一处调用，无对应测试；README 宣称的“表格键盘编辑”实际未被验证。
- 最小修正（不新增依赖、不改架构）：把该逻辑移入 `interaction.rs`（或 `editor.rs` 邻接 `tab_step`），收口为 `Editor::append_table_row(&mut self, block: Range, row_src: &str) -> Option<usize>`：内部 `session.apply` + `self.dirty = true` + `break_undo_group()` + 把 caret set 到新行首单元格偏移；`table_ctrl_enter` 只做定位与调用。
- 回归验证：新增 app_tests 用例断言（a）serialize 含新行且列数正确；（b）`is_dirty()` 为 true 且标题含 `*`；（c）caret 偏移落在新行单元格内而非块首；（d）未保存时 `confirm_close` 触发注入的 Confirmer（选 Save/Cancel 各验一条）；（e）单次 undo 回到加行前；（f）`cargo test --workspace`（期望 902 + 新增）与 `cargo clippy --workspace --all-targets -- -D warnings`；（g）真机：表内 Ctrl+Enter → 关窗弹确认 → 撤销回到原表格。

### 可改进的

- 【中】冲突副本以“读失败即不存在”判定空位（doc_state.rs:521-531）。若既有 `.conflict-n.md` 被占用/无权限，`store.save` 的原子替换会覆盖上一份冲突备份——用户唯一的外部版本被销毁。建议改为“候选名带时间戳/序号单调递增且不探测既有名”，或把 IO 错误分类上移到端口契约。
- 【中】窗口不可见时跳过所有周期工作（app.rs:967-971 及 `about_to_wait`）。最小化期间不写自动保存与恢复快照，若此时崩溃/重启，已编辑内容无恢复副本。属设计取舍，但与“崩溃不丢内容”的产品承诺有张力。
- 【低】`Theme` 派生值存放在 `InputState`（同 4 节），新增主题相关字段时容易被误当作输入再次赋值。

### 反模式

- 【低】`Editor` 同时提供 `session()` 与 `session_mut()`，把“唯一修改入口”降级为约定；【高】1 即该逃逸口的实际使用。

---

## 7. 结论

**架构未达标。** 阻塞项 1 个：【高】1 表格 Ctrl+Enter 绕过 Editor 门面 → 脏标不置位 → 自动保存 / 崩溃恢复 / 关窗确认三条数据安全路径全部失效，且光标膝跳、无测试覆盖。其余 1 高以下问题均为非阻塞债务：禁则两套成形策略、三连提示未收口、全量字数/全文克隆、双入口重置序列、超长函数、公共面与注释漂移。

| 原则 | 评分（5） | 主要依据 |
|---|---|---|
| 第一性原理 | 4.5 | 字节真值/派生可弃、脏块契约；扣分：禁则成形双源 |
| UNIX 哲学 | 4.0 | 单向分层、端口注入、纯函数路由；扣分：paint.rs 承载编辑策略 |
| 二八定律 | 3.8 | 无 GPU/插件、测试集中；扣分：O(n) 字数与全文克隆、表格双趟 |
| 极简主义 | 3.5 | 3 crate / 8 依赖；扣分：双入口重置、公共面膨胀、死代码 |
| 代码整洁之道 | 3.0 | 注释与命名优秀；扣分：292 行 render、三连提示、6856 行测试文件 |
| 软件设计哲学 | 3.3 | 深模块与因果键优秀；扣分：【高】1 逃逸口与脏标契约破坏 |
| **综合** | **3.7** | 达标线建议 ≥4.0 且无“高” |

修复【高】1 并补齐（a）-（g）回归后可判“架构达标（带债务清单）”；在此之前，任何发布门禁都不应把“表格键盘编辑 + 自动保存 + 关窗确认”组合视为已覆盖。

---

## 8. 修复记录（审计后）

**【高】1 已关闭（2026-11-05）**：
- `Editor::append_table_row(block, new_src, caret_in_block)` 收口：`break_undo_group` → `session.apply` → `dirty = true` → `set_caret` 新行首格。
- `paint.rs::table_ctrl_enter` 只做定位与调用；删除 `Editor::session_mut()`，消除逃逸口。
- 回归：`editor::tests::append_table_row_sets_dirty_and_moves_caret_into_new_row`、`app_tests::table_ctrl_enter_sets_dirty_and_moves_caret_into_new_row`（断言脏标、标题 `*`、光标在新行、单撤销步）。
- 门禁：`cargo test --workspace` 全绿、`cargo clippy --workspace --all-targets -- -D warnings` 零警告。

**结论更新**：高严重度阻塞项已清零；剩余【中】【低】债务（冲突副本命名、最小化期自动保存、主题派生值、禁则双源等）不阻塞 v1.0，但列入 v1.1 债务清单。综合评分维持 3.7（审计快照），「架构达标（带债务清单）」的判定条件已满足。

---

## 9. 本次审计未覆盖（避免误解）

- 未重跑基准与红线上限，未跑 `cargo test`；所有性能结论为静态判断（统计/克隆路径），不替代实测。
- 未逐行审计 `veil/src/platform/{wic,print,watch}.rs` 的 Win32 细节，仅核对其在端口契约内（platform/mod.rs:1-241）。
- 未审计 `view/highlight.rs`（1037 行）的语法着色规则正确性，仅确认其属派生层且受缓存键约束。
