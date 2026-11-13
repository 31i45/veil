# Veil 架构与实现技术细节分析 · 对标主流 Markdown 软件差异

> 日期：2026-11-13 · 基线 commit：`main` · 分析性质：静态代码阅读 + 已有 ADR/报告交叉印证 · 零代码改动
> 规模：72 文件 / ~44,442 LOC（core 9,242；ui 31,705；veil 3,495）· 测试：1,459 passed / 0 failed

---

## 第一部分：项目概览与定位

### 1.1 一句话定位

**Veil 是一款纯 Rust、单文件 exe、零外部运行时的 Windows 原生 Markdown 编辑器，单机单文档纯离线，以 Typora 式即时渲染（WYSIWYM）为交互范式，以碾压级体积/内存比为核心差异化。**

### 1.2 关键数字

| 指标 | 数值 | 口径 |
|---|---|---|
| 产物体积 | 7.32 MB（单 exe） | 发布构建，含 PDF/数学/图片渲染 |
| 冷启动 | 145–190 ms（应用内） | `main` → 首帧 |
| 常驻内存 | 40.76 MB | 10 万字真实文档 Private Working Set |
| 击键延迟 p95 | 5.2 ms（10.5k 块） | 进程内全链路探针 |
| 传递依赖 | 117 个 normal crate | workspace |
| 直接依赖 | 8 个 | veil-ui 层 |
| CommonMark 0.31.2 一致性 | A/C/D 维 652/652 · B_EXPORT_EXACT 629/652 | 官方测试集 |
| GFM 扩展一致性 | A/C/D 维 23/23 · B_CANON 12/23 | 官方测试集 |

### 1.3 非目标（显式排除）

- 协作编辑 / CRDT（永久排除）
- 插件系统 / 市场
- 移动端 / PWA
- 任何网络请求、自动更新、遥测
- Mermaid 全量渲染
- 独立富文本文档模型
- 表格合并拆分、列宽拖动
- 多标签 / 多文档窗口（既有决策）
- 拼写检查（闸门四条件）

---

## 第二部分：架构深度分析

### 2.1 总体架构：三 Crate 单向依赖

```
┌─────────────────────────────────────────────────────┐
│  veil（产品二进制）                                   │
│  ─ platform/ Windows FFI                           │
│  ─ PDF 原生导出（pdf-writer + ttf-parser + subsetter） │
│  ─ HTML 导出（pulldown-cmark）                       │
│  ─ 公式渲染（latex-rust STIX Two Math）               │
│  ─ 平台端口注入（Clipboard / FileStore / Math …）      │
└──────────────┬──────────────────────────────────────┘
               │ 单向依赖
┌──────────────▼──────────────────────────────────────┐
│  veil-ui（视图适配层）                                │
│  ─ winit 事件循环 + softbuffer 软件呈现              │
│  ─ cosmic-text 排版 + shaping                        │
│  ─ LayoutEngine（块级布局 + 消隐 + OffsetMap）       │
│  ─ Editor（编辑策略、光标、选区）                     │
│  ─ Chrome 自绘（菜单栏/状态栏/滚动条/弹出菜单）        │
│  ─ Ports trait 聚合（15 个端口，条件注入）              │
│  ─ Memo 因果键缓存（layout / raster / grid / outline） │
│  ─ Persist 状态机（自动保存 + 外部变更 + 崩溃恢复）     │
└──────────────┬──────────────────────────────────────┘
               │ 单向依赖
┌──────────────▼──────────────────────────────────────┐
│  veil-core（纯逻辑内核）                              │
│  ─ Buffer：lapce-xi-rope + Vec<Mark> 自封装          │
│  ─ Session：唯一门面（编辑/撤销/标记查询/块快照）       │
│  ─ Syntax：块级增量解析 + inline 解析 + 派生数据        │
│  ─ History：事务化撤销/重做                           │
│  ─ 零 GUI 依赖 · 零 async 依赖 · 单线程                 │
└─────────────────────────────────────────────────────┘
```

**依赖规则**：反向边禁止（veil-core 不得引用 veil-ui/veil，veil-ui 不得引用 veil 的平台代码）。veil-core 公开面仅 19 项，veil-ui 公共导出 25 项（含 5 项 bench_typing 纯排版基准）。

### 2.2 veil-core：纯逻辑内核

#### 2.2.1 Buffer —— lapce-xi-rope + Mark 自封装

**设计决策（ADR-001）**：
- 文本存储选用 `lapce-xi-rope`（Lapce 编辑器生产 fork，活跃维护），而非 `xi-rope` 原版（上游归档）。
- Mark 语义（bias、偏移变换、注解挂载）在 `veil-core` 内自封装，不向外暴露 rope 类型。
- Mark 存储用 `Vec<Mark>`（按创建顺序，不维护全局有序），每次编辑全量 delta transform，复杂度 O(#marks)。

**Mark 变换规则（ADR-003）**：

```
设删除 [s, e)、插入 n 字节：
- p < s     → 不动
- p > e     → 平移 n - (e - s)
- s < p < e → 坍缩到 s（锚定内容已删）
- p == s    → Left → s；Right → 纯插入时 s+n、删除时坍缩到 s
- p == e    → Right → s+n；Left → s
```

**Mark 数量约束**：当前产品常驻 mark 恒为 2（caret + anchor，光标移动峰值 4）。升级触发条件量化为 ≥ 1,000 常驻 mark、mark 变换占帧预算 > 5%、或 mark 内存 > 1 MB（`debug_assert` 兜底）。

**关键实现**：
- `Buffer::edit(start, end, text)` → `transform_marks(start, end, text.len())` → `rope.edit(start..end, text)` → 返回被删除文本供 undo。
- `prev_grapheme` / `next_grapheme` 由 rope 内置实现，无需额外 `unicode-segmentation` 依赖。

#### 2.2.2 Session —— 唯一门面

**入口模式**：视图层每次交互 ≤ 3 行：`翻译事件 → Session::apply → 按 Events.dirty_blocks 失效重绘`。

```rust
pub enum Edit { Insert { at, text }, Delete { range }, Replace { range, text } }

pub struct Events {
    pub dirty_blocks: Vec<BlockId>,  // 脏块重建；未列出的块保证未变
}
```

**内部管线**（`apply_primitive`）：
```
编辑 → 记录 old_start_line / old_end_line → Buffer::edit(start, end, text) →
Syntax::reparse(buf, old_start, old_end, new_end) → 返回 dirty_blocks →
History::record(Transaction { prim: PrimEdit })
```

**撤销/重做**：同一条路径的逆操作。撤销时应用反向 delta，生成新的反向事务压入 redo 栈。这是 ADR-001 delta 设计的直接收益——undo 层几乎免费。

**可观测接口**：
- `blocks()` —— 结构快照 `(BlockId, BlockKind, Range)`
- `block_inlines(block_id)` —— inline 节点快照
- `line_col(offset)` / `text_slice(range)` / `prev_grapheme(offset)` / `next_grapheme(offset)`
- `undo_depth()` / `redo_depth()` / `block_count()` / `last_rescan_lines()`

#### 2.2.3 Syntax —— 增量解析引擎

**核心思想**：脏块局部重同步。从脏区所在块起点重扫，向后扩展，直到扫描出的块边界与既有块（平移后）在「起始行 + 种类」上重新对齐。对齐点之后的旧块整体保留（id 不变）。

**块级解析（block.rs）**：
- 支持的块类型：ATX/Setext 标题、段落、围栏代码、缩进代码、展示数学 `$$`、GFM 表格、列表项（含 task list 复选框、有序/无序、嵌套深度）、引用块、分隔线、链接引用定义、脚注定义、原始 HTML 块、`<details>` 容器（含 BlockContainer 标注）、Front Matter（YAML/TOML 首块）、GFM Admonition 五种 + 自定义 Callout。

**增量重扫策略**：
1. 从编辑起始行向前回走到块边界（处理围栏代码吞咽、表格头污染等前后关联）。
2. 向后扫描，直到新扫描块的（start_line, BlockKind）与旧块一致（旧块已做 offset 平移）。
3. 该对齐点之后的旧块整体保留，避免全文档重解析。
4. 围栏代码块、Setext 标题、表格头都可能向前/向后污染，由同一对齐规则统一覆盖。

**派生数据（随 reparse 自动重建）**：
- 链接引用表 `RefMap`（label → dest + title）
- 脚注编号表 `id → 编号`（1 起，按首次引用顺序）
- 脚注定义表 `id → 首行内容区间`
- BlockId → blocks 内索引（O(1) 查表）
- Callout 分类（`> [!FIXME]` → 配置里定义的 Callout；未知名保持 BlockQuote）

**Inline 解析（inline.rs）**：
- `**strong**`、`*emph*`、`` `code` ``、`[text](url)`、`![alt](url)`、`$math$`、`~~del~~`、`==highlight==`、`[^id]`、行内白名单 HTML（kbd/sub/sup/br/mark/abbr/img/code/pre/a/strong/em…）。
- 白名单外 HTML 退回字面文本；`javascript:` href、`url()` style、远程图片 src 等危险内容解析层置空。
- 行内节点区间全为文档字节偏移，半开 `[start, end)`，含开闭标记。

#### 2.2.4 History —— 事务化撤销

- 每次编辑产生一个 `Transaction { prim: PrimEdit { start, deleted, inserted } }`。
- 撤销时对原位置应用反向 edit（deleted ↔ inserted 互换），生成新的反向事务压入 redo 栈。
- `break_undo_group()` 显式断开撤销分组，光标移动/选区变化后下一次编辑另起一步（VS Code 语义）。

### 2.3 veil-ui —— 视图适配层

#### 2.3.1 状态五分类模型（ADR-009）

一切 App 状态必须且只能属于五类之一：

| 类别 | 定义 | 允许操作 | 示例 |
|---|---|---|---|
| **Truth** | 唯一来源，重开后仍应存在 | 仅经 Session::apply 或文档命令修改 | 文本 rope、光标/选区、撤销历史、path、bom、磁盘基线 |
| **Input** | 外部喂入，应用只读 | 只读，由事件/注入赋值 | 视口尺寸、scale、主题模式、系统深色、IME 组合串 |
| **Derived** | 由真值 + 输入唯一决定 | 只可重算，不可直接改 | 块索引、布局、光栅、公式图、表格网格、大纲 |
| **Port** | 能力，不携带状态 | 调用 | Clipboard、FileStore、MathRenderer 等 |
| **Transient** | 一次交互内有效 | 事件内读写 | mods、mouse、dragging、goal_x、hover |

唯一派生公式：
```
frame = f(文档真值, 视口, 缩放, 主题, 交互模式, 指针与焦点)
```

**因果键 Memorization**（替代手工失效）：

```rust
struct CacheKey {
    block: BlockId,     // 内容因果：脏块重解析即换新 BlockId，天然失配
    reveal: bool,       // 标记展开影响块内排版与光栅
    width_q: u32,       // 版心宽量化
    scale_q: u32,       // DPI 量化
    theme_rev: u32,     // 主题代次
}
struct Memo<V> { key: CacheKey, value: V }
```

**关键收益**：生产代码 0 处手工失效。`Cache` 的 7 个 `invalidate_*` 方法已全部删除，外部 30 处调用点归零。内存回收由 `prune` 承担（只删已消失 BlockId 与过期代次）。

#### 2.3.2 渲染纯化与呈现分离

```rust
fn render(&mut self) -> &mut Frame   // 产出完整帧缓冲，只读真值，只填派生缓存
fn present(&mut self)                 // 唯一外部副作用出口：surface 拷贝 + present
```

约束：`render` 对真值只读，只允许填充 Derived（memo）；`present` 是唯一产生外部副作用的出口。幂等性成为正确性属性——同一输入连续调用两次 `render`，输出帧逐像素相同。

#### 2.3.3 LayoutEngine —— cosmic-text 排版

**核心管线**：
```
块 Block → 消费 InlineNode 树 → 构建 cosmic-text Buffer（Attrs 叠加）→
OffsetMap（doc↔layout 坐标换算）→ shape_until_scroll →
行高/块高计算 → 写入 Layout Cache（Memo 因果键命中）
```

**WYSIWYM 消隐机制**：
- 非激活 inline 节点的标记段不参与排版（行尾 `\r` 同理）。
- 激活规则 v0：`range.start <= caret <= range.end` 闭区间，逐节点独立、嵌套各自生效。
- 光标进出标记 → reveal 布尔变 → 因果键失配 → 重排 → 标记从"隐"变"显"或反之。
- OffsetMap 负责 doc 字节偏移 ↔ layout 物理像素的双向映射，对外偏移一律为文档坐标。

**排版上限保护**：
- `MAX_BLOCK_BYTES = 64 KiB` / `MAX_BLOCK_LINES = 512` / `MAX_RASTER_PX = 24,000,000`。
- 超限提示明确告知「未渲染」而非静默截断。

**行内资源占位**（Math / Image）：
- 布局阶段只放占位图（宽度/高度/基线），光栅化由独立路径异步完成。
- 图片按文档目录解析后的绝对路径作缓存键；另存为/打开改变路径基准即失配。
- 行内公式按 TeX 源码内容寻址，跨文档天然复用。

#### 2.3.4 Chrome 自绘（ADR-005）

菜单栏/弹出菜单/状态栏/滚动条全部在 `chrome/` 纯逻辑实现（几何、命中、滚动换算、状态文本为纯函数），绘制复用 `Frame` 原语。

**视口化**：内容区 = 窗口减去菜单栏与状态栏；滚动条占右侧 12px；滚轮与滚动条共用 `max_scroll` 钳位。Chrome 最后绘制、不透明覆盖，保证内容不侵入。

**系统文件对话框**：手写 Win32 `GetOpenFileNameW` / `GetSaveFileNameW` FFI（`platform/windows.rs`），零新增依赖。

#### 2.3.5 端口倒置 + Headless 可测（ADR-004, ADR-009）

15 个平台端口 trait 注入，产品层 veil 创建 `Ports` 聚合结构体后传入 `veil_ui::run()`：

| Port | 职责 | 产品层实现 |
|---|---|---|
| Clipboard | 文本/HTML/图片/文件列表 | arboard |
| FileStore | load / save（原子写） / list_files | std::fs |
| Clock | 本地 HH:MM / datetime | 手写 Win32 GetLocalTime |
| MathRenderer | parse → layout → render_png | latex-rust STIX Two Math |
| ImageLoader | 图片解码（PNG→png crate；其余→WIC） | FsImages |
| AssetStore | 粘贴图片落盘到 `<doc>.assets/` | FsAssets |
| Exporter | HTML / PDF 导出 | HtmlExporter + 原生 PDF |
| Dialogs | 系统打开/另存为 | Win32 OPENFILENAMEW |
| Watcher | 外部文件变更监听 | notify crate |
| ThemeEnv | 系统外观跟随、窗口效果 | Win32 DwmSetWindowAttribute |
| PersistEnv | 崩溃恢复快照读写 | JSON + 魔数 VEIL-RECOVERY-1 |
| Print | 打印对话框 + GDI 打印 | Win32 PrintDlg |
| UrlOpener | http/https/mailto 链接 | ShellExecuteW |

**headless 测试**：全部 1,091 个测试中 UI 层 724 个（含 proptest 性质测试钉死 memo/undo 不变量）。Fake 全家桶让 UI 逻辑可在无窗口环境验证。渲染幂等断言确保同一输入产出逐像素相同帧。

#### 2.3.6 Persist 状态机 —— 不丢稿的结构保证

**自动保存触发条件**：`dirty && has_path && !ime_active && !hold && idle`（`autosave.rs`）。

**外部变更处理**：
- 外部文件被修改 → `notify` 事件线程入队 → 主线程 `tick()` 消费 → `Persist::begin_external()` → 挂起自动保存 → 显示三选一模态（覆盖 / 备份后再看 / 保持草稿）。
- 外部文件被删除 / 不可读 / 变为非 UTF-8 → 各有独立文案与恢复动作。

**崩溃恢复**：启动时按 `VEIL-RECOVERY-1` 魔数读取 `<path>.veil-recovery` 快照，三级决议优先级：崩溃恢复 > 会话恢复 > 传入文档。

### 2.4 veil —— 产品二进制

- **平台接缝**：条件编译 `#[cfg(target_os = "windows")] mod imp`，上层零 `#[cfg]`，签名一致性由编译器强制。macOS/Linux 实现故意缺失 → 编译失败 → 强制显式决策。
- **单实例 IPC**：WM_COPYDATA + 命名事件，主实例监听、次实例发送 `NewWindow` 请求。
- **PDF 原生导出**：pdf-writer 0.15 + ttf-parser + subsetter 0.2.6。多级书签、CJK 字体子集化、heading/table/image/math 布局。兜底 Microsoft Print to PDF。
- **HTML 导出**：pulldown-cmark 解析 → 内嵌 CSS + 数学 SVG（latex-rust）→ 自包含单文件。
- **图标/资源**：exe 内嵌图标，无 vendor 外部资产。

### 2.5 构建与体积纪律

```toml
[profile.release]
lto = true          # 链接时优化
codegen-units = 1   # 单 codegen unit 保证 LTO 全量
panic = "abort"     # 无 panic 展开

# 硬门禁
依赖黑名单：tokio / wasmtime / webview2-com / wry
直接依赖红线：≤ 8
传递依赖红线：≤ 200
产物体积：7 MiB（Wave 4 放宽后的红线）
```

---

## 第三部分：关键技术实现细节

### 3.1 文本编辑管线全链路

```
用户按键 → winit WindowEvent::KeyDown →
App::on_key(...) → Editor::translate(...) → mode.route(key, mods, composing) →
Session::apply(Edit::Insert { at, text }) →
  [veil-core]
  ├─ Session::check_bounds → Buffer::edit(start, end, text)
  │   └─ Buffer::transform_marks → rope.edit → 返回 deleted
  ├─ Syntax::reparse(buf, old_start_line, old_end_line, new_end_line)
  │   └─ 向前回走到块起点 → 向后扫描对齐 → 生成 dirty_blocks
  └─ History::record(Transaction)
→ Events { dirty_blocks: vec![...] } →
  [veil-ui]
  ├─ BlockIndex::rebuild(dirty) →
  ├─ LayoutEngine::layout(block_id, cache_key) →
  │   └─ Memo<Layout>::hit_or_compute(cache_key) → cosmic-text shape → 光栅
  ├─ Editor::update_caret(mark_offset) → BlockId 定位 →
  └─ render() → Frame（逐块 blit） → present() → softbuffer::present
```

### 3.2 增量解析的"脏块重同步"算法

**问题**：编辑可能影响前后关联块（围栏代码块的起始行、表格头、Setext 标题的下划线）。如果只重扫编辑所在行，后续块可能因行号偏移失配。

**算法**（来自 `syntax/mod.rs` 的 reparse）：

1. 计算编辑影响的行范围：`[old_start_line, new_end_line)`。
2. **向前回走**：从 `old_start_line` 向前逐行回走，直到遇到明确的块边界（空行、块起始标记、或文档头）。这处理围栏代码吞咽、表格头被污染等。
3. **向后扫描**：从回走点开始重新扫描块，同时维护旧块列表（已做 offset 平移）。
4. **对齐检测**：新扫描出的块 `(start_line, BlockKind)` 与旧列表中对应块逐行比对。一旦一致 → 对齐点确定 → 该点之后的旧块 **整体保留**（id 不变、offset 已平移）。
5. **返回脏块**：从回走点到对齐点之间的块 id 全部标记 dirty。

**性能特性**：
- 行内编辑（在一个段落中插一个字符）：重扫行数通常 = 该段落的行数（通常 1–5 行）。
- 跨块编辑（删除段落间空行）：回走到上块起点、向后扫到重新对齐，典型 10–30 行。
- 10 万字文档（~2000 段落）、中间段落编辑：reparse ≈ 几十行，p95 约 6.78 ms（5,005 块 fixture 实测）。

### 3.3 数学渲染管线

```
用户输入 $$...$$ 或 $...$
  ↓
veil-core 解析 → BlockKind::Math / InlineKind::Math
  ↓
layout 阶段：content 区间 → 占位 InlineImg { width, height, baseline }
  ↓
光栅化阶段（异步，Cache 键 = TeX 源码 + scale_q + theme_rev）：
  latex_rust::parse(tex) → MathAst →
  latex_rust::layout_with_em_size_pt(ast, font, style, em) → MathBox { height, depth, width } →
  latex_rust::render_png(tree, font, PngOptions) → PNG 字节 →
  png::Decoder → RGBA8 ImageData → baseline = height(em) × em_px
  ↓
绘制：Frame::blit_png(x, y, ImageData) —— 行内与文本基线对齐
```

**降级策略**：latex-rust 初始化失败 → NoMath 渲染器恒 Err → 所有数学块显示源码（不静默失败、不崩溃）。

### 3.4 安全过滤

**白名单 HTML 标签**（`HtmlTag` 枚举，26 种）：kbd, sub, sup, br, mark, abbr, img, code, pre, a, strong, b, em, i, u, s, del, ins, q, cite, var, samp, small, span, details, summary。

**危险标签全剥**（`HTML_STRIP_TAGS`）：script, style, iframe, object, embed, form, textarea。

**危险属性/值置空**：`javascript:` href、远程（http/https）图片 src、`url()` / `expression` / `behavior` / `z-index` / `content` style 值。

**style 属性净化**：仅允许 color, background-color, font-weight, font-style, text-decoration, font-size（8–72px）, font-family（含 mono 关键词）。CSS 颜色支持 hex（3/6）、rgb/rgba、hsl/hsla、有限命名色白名单（45 种）。

### 3.5 表格编辑

- GFM 表格：block 级 `BlockKind::Table` → `Session::table(block_id)` 返回 `Table { aligns, header, rows }`。
- 活跃单元格进入源码态（显示 Markdown 源），其余单元格渲染（bold/italic/code/links 标记隐藏）。
- Tab / Shift+Tab 导航单元格（最后 wrap 回第一）；Ctrl+Enter 尾部追加空行。
- 行/列插入/删除、列对齐设置：右键菜单触发 → 一个 undo step。
- 分隔行 `:--` / `--:` / `:-:` 解析为 CellAlign（Left/Center/Right）。

---

## 第四部分：与主流 Markdown 软件的差异对比

### 4.1 竞品画像与技术栈一览

| 软件 | 技术栈 | 交互范式 | 体积 | 内存 | 平台 | 定价 |
|---|---|---|---|---|---|---|
| **Veil** | Rust · winit + softbuffer + cosmic-text · 单 exe | 即时渲染 WYSIWYM | **7.32 MB** | **40.76 MB** | Windows 11 x64 | 免费 MIT |
| **Typora** | Electron（Chromium + Node.js） | 即时渲染 WYSIWYM | ~200+ MB（未取原文） | 数百 MB | Win/Mac/Linux | $14.99 买断 |
| **MarkText** | Electron（MIT 开源） | 即时渲染 + 源码模式 | **488.95 MB** | **365.01 MB** | Win/Mac/Linux | 免费 MIT |
| **Typedown** | WinUI 3 / WebView2 | 即时渲染 WYSIWYM | 176.56 MB（Appx） | 数百 MB | Windows Store | 免费 |
| **Zettlr** | Electron（GPLv3） | 库式 + 双栏编辑/预览 | 数百 MB | 数百 MB | Win/Mac/Linux | 免费 + 赞助 |
| **ghostwriter** | Qt（GPLv3） | 双栏左编辑/右预览 | 中等 | 中等 | Win/Mac/Linux | 免费 |
| **VS Code** | Electron + Monaco | 源码 + Markdown All in One | ~100+ MB | 数百 MB | 全平台 | 免费 |
| **iA Writer** | 原生（AppKit/Cocoa） | 单栏纯文本 + 极轻高亮 | 中等 | 低 | Win/Mac/iPadOS | $买断（分平台） |
| **Obsidian** | Electron | 库式 + 双栏 | 中等 | 中等 | 全平台 | 免费 + 订阅 |

### 4.2 架构层对比

#### 4.2.1 渲染架构：软件合成 vs GPU/WebView

| 维度 | Veil | Typora/MarkText/Typedown | VS Code | iA Writer |
|---|---|---|---|---|
| **渲染后端** | softbuffer（软件合成） | Chromium GPU 进程 | Chromium GPU + Monaco 文本层 | AppKit / Cocoa 原生 |
| **文本排版** | cosmic-text（纯 Rust shaping + font fallback） | HarfBuzz + Skia | Monaco（自研文本布局） | Core Text / DirectWrite |
| **Chrome UI** | 自绘（Frame 矩形/位图原语） | Electron HTML/CSS | Electron HTML/CSS | 系统原生控件 |
| **GPU 依赖** | **零**（wgpu 已触发式延迟） | Chromium GPU 进程 | Chromium GPU 进程 | 可选 Metal |
| **单进程/多进程** | 单进程 | 多进程（main/renderer/GPU） | 多进程（main/renderer/GPU/extension） | 单进程 |

**Veil 的极端性**：在所有竞品中是唯一零 GPU 依赖、零 WebView、零异步运行时的方案。代价是无法利用硬件加速的动画/过渡效果，但换来的是：
- **体积**：对 MarkText 碾压 67×（7.32 vs 489 MB）
- **内存**：碾压 9×（40.76 vs 365 MB）
- **冷启动**：进程创建后 30 ms 即窗口句柄就绪

#### 4.2.2 文本模型

| 软件 | 文本存储 | 编辑模型 | 增量解析 |
|---|---|---|---|
| **Veil** | lapce-xi-rope（带 delta + mark transform） | 原语 Edit → Buffer::edit + Syntax::reparse | 脏块局部重同步（回走→扫描→对齐） |
| **Typora** | 未公开（Electron 内通常是 JS string 或自实现） | 事件驱动整块重渲染 | 未公开 |
| **VS Code** | 自实现 Piece Tree（Rust port） | Piece Tree insert/delete | 语法树增量（TextMate / Semantic Tokens） |
| **Zettlr** | 同 Electron 家族 | 事件驱动 | 未公开 |

**Veil 的独特性**：
1. **Delta 模型自封装**：rope 的 delta transform 同时服务于 mark 跟随 + undo 逆操作，一层逻辑两个用途。
2. **块级增量边界**：不做字符级/行级重新解析，以块为最小失效单位。块内 inline 节点按需解析（`block_inlines` 懒计算）。
3. **序列化恒等于原文**：`serialize()` 直接返回 rope 的完整文本，不经过 AST → Markdown 生成。这保证了 **字节往返 100%**——打开→编辑→保存，未变部分一个字节不动。

#### 4.2.3 状态管理

| 软件 | 状态模型 | 缓存失效 |
|---|---|---|
| **Veil** | 五分类（Truth/Input/Derived/Port/Transient） | 因果键 Memo（键失配即重算，零手工失效） |
| **Typora** | 未公开（事件驱动） | 未公开 |
| **VS Code** | 复杂（多 Services + EventBus + Decorations） | 手工失效 + generation counter |
| **MarkText** | 未公开 | 未公开 |

**Veil 的结构性优势**：因果键模型使得"忘调失效"在结构上不可能发生。BlockId 会话内单调递增 → 脏块重解析必然换新 BlockId → 所有依赖该 BlockId 的 Memo 自动失配。这在全行业编辑器中属于罕见的正确抽象。

### 4.3 功能层对比

#### 4.3.1 Markdown 规范一致性

| 软件 | CommonMark 合规 | GFM 扩展 | 自定义方言 |
|---|---|---|---|
| **Veil** | A/C/D 维 652/652 · B_EXACT 629/652 · B_CANON 630/652 | Tables/TaskList/Strikethrough 23/23 | Front Matter、自定义 Callout |
| **Typora** | 接近但非官方认证 | Tables/TaskList/Strikethrough/Footnotes | Typora 专有标记扩展 |
| **MarkText** | 接近但非官方认证 | 完整 GFM | 图表（Mermaid/KaTeX）扩展 |
| **Zettlr** | Pandoc 管线（完整但有转换） | Pandoc 支持 | Zk 链接 `[[...]]`、标签 |
| **Ulysses** | Markdown XL（非标准） | — | Ulysses 专有方言 |
| **Obsidian** | 标准 CommonMark | 部分 GFM | `[[...]]` 双链、`==高亮==` |

**关键注释**：Veil 的 B 维（字节往返精确性）不是全通过（629/652），但 A/C/D 维（解析一致性、结构一致性）全绿。这是**编辑架构选择**的结果——Veil 不做 AST → Markdown 生成，serialize 直接返回原文，所以 B 维"canonical form"概念本身不适用。

#### 4.3.2 所见即所得（WYSIWYM / What You See Is What You Mean）

| 软件 | 标记消隐策略 | 光标进出行为 |
|---|---|---|
| **Typora** | 光标进入块 → 显示源码标记；离开 → 完全隐藏、渲染为富文本 | 进出零跳动（核心体验，经过多年打磨） |
| **Veil** | 激活 inline 节点（`range.start <= caret <= range.end`）保留标记、未激活隐藏 | WYSIWYM 标记消隐保行高，光标进出零跳动 |
| **MarkText** | 同 Typora 范式 | 同 Typora |
| **VS Code** | 源码模式显示所有标记；预览模式独立渲染面板 | 预览模式无光标（只读） |

**Veil vs Typora 的差异化**：两者共享同一交互范式，但 Typora 块级策略（整块进/整块出），Veil inline 级策略（逐节点独立激活）。Veil 的激活规则更细粒度，但带来的实现复杂度也更高——需要 OffsetMap 维护 doc↔layout 坐标双向映射。

#### 4.3.3 高频编辑手感

| 能力 | Veil | Typora | MarkText | VS Code |
|---|---|---|---|---|
| 括号/引号/标记自动配对 | ❌ | ✅ | ✅ | ✅ |
| 行操作（上移/下移/复制行） | ❌ | ✅ | ✅ | ✅ |
| 查找替换（大小写/全词） | ⚠️ 仅替换 | ✅ 完整 | ✅ 完整 | ✅ 完整 + 正则 |
| 查找替换（正则） | ❌（体积红线否决） | ✅ | ✅ | ✅ |
| PgUp/PgDn 翻页 | ❌ | ✅ | ✅ | ✅ |
| Ctrl+Backspace/Delete 按词删除 | ❌ | ✅ | ✅ | ✅ |
| 表格活跃单元格行内编辑 | ✅ | ✅ | ✅ | ❌（插件实现） |
| 代码块语法高亮 | ✅ 自研状态机（16 语言） | ✅ 完整 | ✅ | ✅ 完整 |
| 数学渲染 | ✅ latex-rust 纯 Rust | ✅ MathJax/KaTeX | ✅ KaTeX | ✅（插件 KaTeX） |
| 图片粘贴 | ✅ | ✅ | ✅ | ✅ |
| 中文排版（禁则/WJ/UAX#14） | ✅ 5/5 隐藏王牌 | 一般 | 一般 | 一般 |

**Veil 的关键短板**：7 项高频编辑手感在竞品中属标配，但 Veil 全库无对应实现路径。gap-analysis 报告将这 7 项列为 P0 体验债，合计增量仅 8–15 KiB（远在体积红线内）。

#### 4.3.4 导出能力

| 格式 | Veil | Typora | MarkText | Zettlr |
|---|---|---|---|---|
| HTML（自包含） | ✅ | ✅ | ✅ | ✅（Pandoc） |
| PDF（原生） | ✅ 原生 pdf-writer + 字体子集化 + TOC | ✅ 打印管线 | ✅ 打印管线 | ✅ Pandoc |
| PDF（Microsoft Print to PDF 兜底） | ✅ | ❌（Chromium 打印） | ❌（Chromium 打印） | ❌ |
| Word (.docx) | ❌ | ✅ | ✅ | ✅ Pandoc |
| LaTeX | ❌ | ✅ | ✅ | ✅ Pandoc |
| ePub | ❌ | ✅ | ❌ | ✅ Pandoc |
| 图片 (.png) | ❌ | ✅ | ✅ | ❌ |

**Veil 的导出定位**：只做 HTML 和 PDF，拒绝 Pandoc 管线（避免引入 100+ MB 外部依赖）。PDF 是自己手写的原生实现（pdf-writer + ttf-parser + subsetter），多级书签 + CJK 字体子集化 + heading/table/image/math 全布局。Microsoft Print to PDF 兜底保证任何情况下都能导出。

### 4.4 UX 层对比

#### 4.4.1 信任与不丢稿

| 机制 | Veil | Typora | MarkText | VS Code |
|---|---|---|---|---|
| 自动保存 | ✅ 五条件（dirty/path/!ime/!hold/idle） | ✅ | ✅ | ✅ |
| 崩溃恢复 | ✅ 三级决议（崩溃快照 > 会话 > 传入） | ✅ | ✅ | ✅ HotExit |
| 外部修改通知 | ✅ 三选一模态（覆盖/备份/保持） | ✅ | ✅ | ✅ |
| 自动保存反馈 | ⚠️ 沉默（仅标题栏 `*` 消失） | ✅ 显性化 | ✅ | ✅ |
| 字节往返 100% | ✅（serialize 直接返回原文） | ⚠️（可能改动） | ⚠️ | ❌（语法高亮/格式化会改） |

**Veil 的"不丢稿"是真正的护城河**（gap-analysis 评 5/5），但**唯一短板是反馈沉默**——自动保存发生时用户唯一信号是标题栏 `*` 消失。这在信任机制上构成不对称：机制强于 Typora，感知却弱于 Typora。

#### 4.4.2 设计约束

| 约束 | Veil | Typora | MarkText | iA Writer |
|---|---|---|---|---|
| 行长上限 | ❌（2560 px 下单行 ~150 中文字） | ✅ 约 70 字符 | ✅ | ✅ 强约束 |
| 标题层级分离 | ⚠️ H4=H5=H6 同为 1.1 倍 | ✅ 明显 | ✅ | ✅ |
| 悬挂缩进 | ❌ | ✅ | ✅ | ✅ |
| 中文排版（禁则/WJ/UAX#14） | ✅ 5/5（隐藏王牌） | 一般 | 一般 | 一般 |
| 光标闪烁 | ❌ 常亮 | ✅ | ✅ | ✅ |
| 全局过渡动效 | ❌ 零过渡 | ✅ | ✅ | ✅ |

### 4.5 反目标分析：Veil 选择"不做"的事

Veil 有一个罕见的设计纪律——明确列出并公开解释了 10 类永久排除项。这在编辑器行业中非常少见：

| 反目标 | 拒绝理由 | 竞品做法 |
|---|---|---|
| **协作编辑 / CRDT** | 单用户定位会牵动文档模型、崩溃恢复与外部监听全部假设 | VS Code Live Share / Typora Cloud / Obsidian Sync |
| **插件市场** | 破坏单文件 exe 与确定性行为 | VS Code / Obsidian / MarkText |
| **移动端 / PWA** | 架构差异大、无需求 | iA Writer / Obsidian / Typora（未做移动端但做 Web） |
| **任何网络请求** | "链接交系统浏览器，自身不发请求" | Typora / MarkText（零网络）/ VS Code（零网络） |
| **自动更新 / 遥测** | 任何网络访问均违规 | 多数竞品有 |
| **Mermaid 全量渲染** | 渲染引擎体积 + 维护成本超 6 MiB 预算 | MarkText / Typora（内嵌）/ VS Code（插件） |
| **独立富文本文档模型** | 正文始终是 Markdown，Ctrl+/ 是视图切换 | 无竞品独立文档模型；Typora 用 AST 驱动渲染但源码仍是 Markdown |
| **安装器 / 文件关联注册** | 便携单文件是核心卖点，关联 opt-in `/register` | 所有竞品有安装器 |
| **表格合并/拆分、列宽拖动** | GFM 无法表达，不扩自有语法 | Typora / MarkText 用自扩展语法 |
| **拼写检查** | 闸门四条件（ISpellChecker 可用性 / 词典 / 体积增量 / 口径冲突） | 多数竞品内建（Typora 弱） |

**关键洞察**：Veil 的拒绝列表不是"还没做"，而是"永远不做"。这使得它的产品边界极其清晰——单机、单文档、离线、便携。这种纪律在行业中独一无二。

### 4.6 同机性能对照（2026-10-30 P2 基准）

| 指标 | Veil | MarkText | 倍率 | 口径 |
|---|---|---|---|---|
| 冷启动（窗口句柄） | 30 ms | 30 ms | 1.0× | Windows PowerShell，5 轮中位 |
| 冷启动（应用内首帧） | 145–190 ms | 未测 | — | VEIL_BOOT_LOG 分段探针 |
| 常驻内存 | **21.51 MB** | 365.01 MB | **17.0×** | 私有工作集，进程树求和 |
| 安装产物体积 | **5.46 MB** | 488.95 MB | **89.6×** | 单 exe vs 安装目录 |
| 击键延迟 p95 | **6.83 ms**（10 万字） | 未测 | — | 进程内全链路探针 |

**诚实声明**：冷启动"打平"不可尽信为 Veil 快。窗口句柄出现时 Electron 的 renderer 通常尚未绘制内容；Veil 窗口句柄出现即接近首帧。该口径系统性偏袒 MarkText，故 30 ms = 30 ms 应读作"Veil 不慢于对手"。

### 4.7 Veil 有而主流 Markdown 软件没有的

| 能力 | 说明 |
|---|---|
| **因果键 Memo 零手工失效** | 结构性消灭"忘调失效"类缺陷（生产代码 0 处）。竞品用 generation counter / 手工失效。 |
| **字节往返 100%** | 打开→编辑→保存，未变部分一个字节不动。AST 驱动渲染的编辑器（Typora/MarkText）可能微调格式。 |
| **纯 Rust 数学渲染** | latex-rust STIX Two Math，无 KaTeX/MathJax 依赖。竞品依赖 JS 引擎。 |
| **自定义 Callout** | `> [!FIXME]` 自定义提示块，解析期分类、渲染期消费。竞品只有五种内置 GFM Alerts。 |
| **PDF 原生导出 + CJK 字体子集化** | 完全手写（pdf-writer + ttf-parser + subsetter），不依赖 Chromium 打印或 Pandoc。 |
| **Microsoft Print to PDF 兜底** | 任何情况下都能导出 PDF，竞品走 Chromium 打印管线（可能受驱动影响）。 |
| **单实例 IPC + 新窗口** | WM_COPYDATA 主/次进程通信，次进程发送 NewWindow 请求。 |
| **崩溃恢复魔数校验** | `VEIL-RECOVERY-1`，启动时可验证快照完整性。 |
| **中文排版禁则 + WJ 胶水 + UAX#14** | 行首 24 + 行尾 11 字符禁则表、悬挂标点、整体下沉——超过 Typora/iA Writer 公开能力描述。 |
| **零网络请求 / 零遥测 / 零自动更新** | 链接交系统浏览器，自身不发任何请求。竞品多数有自动更新。 |

### 4.8 主流 Markdown 软件有而 Veil 没有的

| 能力 | 竞品 | Veil 分类 | 理由 |
|---|---|---|---|
| 括号/引号/标记自动配对 | Typora/MarkText/VS Code | **① 真差距（P0）** | 高频、低成本、不新增依赖 |
| 行操作 | Typora/MarkText/VS Code | **① 真差距（P0）** | 同上 |
| 查找替换正则 | VS Code/Sublime | **待决** | 自研子集，禁引引擎（+250–400 KiB 超红线） |
| 查找大小写/全词 | Typora/MarkText | **① 真差距（P0）** | 恒模糊，改默认值即可 |
| DOCX/ODT/LaTeX/ePub 导出 | Pandoc 管线类 | **① 真差距（中频）** | 不引入 Pandoc（100+ MB），原生手写成本高 |
| 插件系统 | VS Code/Obsidian/MarkText | **④ 反目标** | 破坏单文件 exe 确定性 |
| 协作编辑 | VS Code/Obsidian | **④ 反目标** | 牵动文档模型全部假设 |
| 拼写检查 | 多数竞品 | **③ 既有决策不做** | 四闸门：ISpellChecker / 词典 / 体积增量 / 口径冲突 |
| 多标签/多文档 | Typora/MarkText | **③ 既有决策不做** | 单文档定位 |
| 设置 UI（手编 config.toml） | 所有竞品 | **① 真差距（中频）** | 体积增量 + 工作量 |
| 标题折叠 | Obsidian/VS Code | **① 真差距（中频）** | 全库无折叠状态/入口 |
| 平滑滚动/惯性 | 所有竞品 | **① 真差距（中频）** | 当前直改 scroll_y |
| Mermaid 全量渲染 | MarkText/Typora | **④ 反目标** | 渲染引擎体积超 6 MiB 预算 |
| 移动端 | iA Writer/Obsidian | **④ 反目标** | 架构差异大 |
| 表格合并/拆分/列宽 | Typora/MarkText | **④ 反目标** | GFM 无法表达，不扩语法 |

---

## 第五部分：总结与判断

### 5.1 Veil 的核心差异化在哪些层

**体积/内存层**（碾压级，无竞品可及）：7.32 MB / 40.76 MB vs Electron 系普遍数百 MB。这是架构选择的直接收益——纯 Rust、零 GPU 依赖、零 WebView、零异步运行时。

**工程质量层**（行业少见的正确抽象）：因果键 Memo 让"忘调失效"结构性不存在；五分类状态模型让"状态可在哪里改"一句话讲清；端口倒置 + headless 测试让 UI 逻辑可在无窗口环境验证。

**信任机制层**（5/5 护城河，但反馈沉默）：自动保存五条件 + 崩溃恢复魔数校验 + 外部修改三选一 + 字节往返 100%。机制够强、感知却弱。

**中文排版层**（隐藏王牌，5/5 但零对外表达）：行首 24 + 行尾 11 禁则表、WJ 胶水、UAX#14、悬挂标点 v0。超过 Typora/iA Writer 的公开能力描述，但从未对外表达。

### 5.2 Veil 的系统性短板在哪些层

**高频编辑手感层**（7 项真差距，集中在 P0 体验债）：自动配对、行操作、查找精度、翻页键、按词删除、替换键盘可达、光标闪烁。这 7 项不改变产品定位、不触碰体积红线、不引入新依赖，却能改变专业用户第一周的判断。

**表达层**（三处需要明确口径的声明）：
1. 冷启动必须双指标并列（145–190 ms 应用内 vs 30 ms 窗口句柄），不作"快"的卖点。
2. CommonMark 一致性必须带维度后缀（A/C/D 652/652、B 629/652），不得说"全通过"。
3. 体积对比必须同口径（单文件 exe vs 单文件 exe；安装目录 vs 安装目录）。

**审美层极化**：中文排版 5/5，动效 2/5（全局零过渡），行长无上限（2560 px 下单行 ~150 中文字），标题层级压平（H4=H5=H6 同 1.1 倍）。同一个维度里同时存在王牌与硬伤。

### 5.3 为什么 Veil 的架构选择在行业中罕见

1. **单文件 exe + 离线 + Windows-only** 三约束叠加，天然排除了 Electron/WebView/Qt/CEF。在行业趋势上逆流而行。
2. **零 GPU 依赖**的 present day，wgpu/skia/vello 的总增量未实测就拒绝，把硬件加速动画的可能性留给"四条触发条件全部 trip"的未来。
3. **零插件系统**的决定在编辑器行业中极其罕见——VS Code / Obsidian / MarkText 全部走插件化路线。Veil 选择用"确定性行为 + 单文件 exe"作为护城河。
4. **纯 Rust 自绘 chrome**（菜单/状态栏/滚动条/弹出菜单）——既不引入 egui（需要 GL 上下文），也不引入 rfd（跨平台对话框 crate 带 GTK/xdg-portal 传递树），选择手写 Win32 FFI。
5. **自研正则子集或零正则**——拒绝 `regex` crate（+250–400 KiB 超红线），即使这是编辑器标配能力。

### 5.4 对"顶流"Markdown 软件的判断

**Typora**：行业审美天花板，但闭源、不可扩展、大文档有性能口碑问题。Veil 在排版精度（中文禁则）和工程质量（因果键、字节往返）上可能超过 Typora，但 Typora 的五年打磨换来的交互细节成熟度是 Veil v1.2 暂时无法追上的。

**MarkText**：开源阵营最接近 Typora 的实现，但 Electron 代价巨大（489 MB / 365 MB 内存）、维护活跃度不明。Veil 在体积/内存上是碾压级优势，但功能覆盖度上二者相近。

**VS Code + Markdown All in One**：最强编辑基础设施（Monaco 文本引擎、正则查找、语法高亮、插件生态），但原生 Markdown 渲染是弱项，富内容支持依赖插件管线。Veil 在即时渲染体验上是 VS Code 插件永远无法追上的。

**Obsidian**：库式 + 双链笔记路线，与 Veil 单文档定位完全不重叠。

**iA Writer**：极简写作路线，放弃功能换专注，与 Veil 丰富功能集有路线差异。

### 5.5 最终判断

Veil 的架构选择不是"还没做到"，而是"刻意不做"。它选择了一条极其狭窄的路线：**单机单文档纯离线 Windows Markdown 编辑器，以极致体积/内存比和确定性工程质量为卖点**。

在这条狭窄路线上，Veil 做到了极致——7.32 MB、40.76 MB、字节往返 100%、因果键零失效、纯 Rust PDF 原生导出、中文排版禁则。

差距也集中在这条路线允许暴露的层——7 项高频编辑手感（不碰体积红线）、表达口径（诚实声明）、审美统一（改数值加定时器）。这不是"功能不全"，而是"工程债可量化"。

---

## 附录：ADR 索引（11 份）

| ADR | 标题 | 核心决策 |
|---|---|---|
| 001 | rope 选型 | lapce-xi-rope + mark 自封装，Vec<Mark> O(N) 变换 |
| 002 | 公式渲染 | latex-rust STIX Two Math，纯 Rust 排版+光栅化 |
| 003 | mark bias/affinity | 五规则变换 + 随机编辑序列回归测试 |
| 004 | 平台接缝 | 条件编译同名模块，上层零 cfg |
| 005 | chrome 自绘 | 菜单栏/状态栏/滚动条自绘，不引入 egui/GL |
| 006 | 外部文件监听 | notify + 非阻塞通道，300 ms 轮询 |
| 007 | 配置与主题 | 顶层 mode 单一来源，窗口效果 DWM FFI |
| 008 | 依赖升级纪律 | 单 commit、可 revert、体积增量须记账 |
| 009 | 状态模型极致化 | 五分类 + 因果键 Memo + Ports 聚合 |
| 010 | 协作位置 v2 | v1 冻结，v2 立项 |
| 011 | wgpu 触发式延迟 | 四条条件全部 trip 才立项 |

---

*本分析报告由 Trae 生成于 2026-11-13，基于对 Veil 仓库的静态代码阅读 + 已有 ADR/评审报告交叉印证。零代码改动。*
