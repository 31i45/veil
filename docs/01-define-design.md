# Skill 01 产出 · 定义 + 设计（Veil）

> 有效期：需求或架构变更时需更新
> 最后验证：2026-10-24（定义完成门通过；本文为 v1 设计基线）
> 引用约定：数字红线、依赖预算、接缝清单、输入点清单一律以本地 `.skills/INSTANCE.md` 为唯一来源（该目录不入库），本文不复制数值，只引用条款。

---

## A. 定义

### A1 问题陈述

为 Markdown 重度作者，以光标展开标记实现源码即真值。（27 字）

### A2 硬约束

| 类别 | 必须 | 禁止 | 验证方式 |
|---|---|---|---|
| 真值 | 无编辑时 serialize 输出 == 输入字节 | 任何渲染/注解状态写入 .md | INSTANCE §3 幂等不变量属性测试 |
| 依赖 | veil-core 零 GUI、零异步运行时依赖 | tokio / wasmtime / 任何 webview 进入任何 crate | `cargo tree` CI 门禁（INSTANCE §2、§6 黑名单） |
| 平台 | v1 Windows 11 真机全绿才准发布 | 未经真机验证声明跨平台 | INSTANCE §1 场景表 |
| 并发 | 内核单线程；后台任务经消息队列回主线程 | 跨线程直接修改内核状态 | 架构评审 + 内核状态仅经 `Session::apply` 变更 |

### A3 成功标准

性能与规模红线：见 INSTANCE §2（冷启动 ≤300ms、击键→上屏 p95 ≤16ms@10万字、内存 ≤150MB、体积 ≤15MiB、测试 ≥80、往返一致率 ==100%）。

行为标准（补充 INSTANCE 未覆盖的交互验收）：

1. 光标进入/离开 inline 节点，标记展开/收起时 caret 视觉 x 坐标不跳变（goal-column 机制）。
2. IME preedit 下划线显示、候选框跟随光标（INSTANCE §1 抽检 ①）。
3. inline 边界输入行为符合 ADR-003 规则表，全部用例有内核测试。

### A4 非目标

1. wgpu 裸写视图（远期可选后端，不纳入主版本里程碑）
2. 协作编辑 / CRDT
3. 插件系统
4. 移动端 / Web 版本
5. 表格可视化编辑（v1 表格仅渲染 + 源码编辑）
6. Mermaid 内嵌渲染（ADR-002）
7. 任何网络功能（同步、自动更新、遥测）

### A5 风险假设

| 风险 | 验证方式 | 缓解措施 |
|---|---|---|
| rope 依赖维护停滞 | `cargo search` 版本号已核（xi-rope 0.3.0 上游停更、lapce-xi-rope 0.4.0 活跃 fork）；实现期复查更新日期 | ADR-001：选活跃 fork + vendor 退路；mark 语义在 veil-core 自封装，rope 可替换 |
| egui IME 输入抽象不支持自绘 preedit | ✅ 已验证（2026-10-24 spike，本地原型 `winit-ime`（已清理），winit 0.30 + 微软拼音真机人工确认）：`Ime::Preedit` 逐击键送达（组合串 + 光标字节区间），`Ime::Commit` 文本正确，`set_ime_cursor_area` 候选框跟随正确 | 风险解除：绕开 egui 输入抽象、在 winit 层直收 IME 事件的路径成立（INSTANCE §7 接缝不变） |
| JS 引擎跑 KaTeX 首屏性能不足（公式密集文档） | ✅ 已验证（2026-10-24 spike，本地原型 `katex-timing`（已清理））：`katex` crate 在 windows-msvc 不可用（quick-js 后端不支持该 target；duktape 后端缺 `Array.prototype.fill`，矩阵环境渲染失败），主路径改为 `rquickjs` 直集成（ADR-002 已修订）。实测：引擎冷启动 29ms，100 公式 210ms（均值 2.1ms/个） | 100 公式同步渲染占 70% 冷启动预算 → 异步渲染 + `BlockRenderCacheId` 缓存确认为必要设计，首屏禁止同步阻塞 |

---

## 定义完成门检查

- [x] A1 ≤30 字且说清谁+问题+方式+效果
- [x] A2 每条可验证（均有命令/测试/场景表对应）
- [x] A3 每条可测量（数值引用 INSTANCE §2，行为标准有可判定准则）
- [x] A4 共 7 条非目标（≥3）
- [x] A5 3 个风险均有验证方式

---

## B. 设计

### B1 模块边界

crate 级（单向无环）：

```
veil（产品层：标签/文件/剪贴板/导出/设置/大纲）
  → veil-ui（视图适配器：事件翻译、cosmic-text 布局、交互策略、虚拟滚动）
    → veil-core（纯逻辑内核）
```

veil-core 内部（均不对外公开，唯一公开项为 facade）：

```
session（facade：Session::apply(Edit) -> Events）
  ├─ buffer   （rope 文本存储 + mark 存储，delta transform）
  │    → range（Mark{bias}、Range）
  ├── annotate（注解集合：Reveal / BlockRenderCacheId）
  │    → range
  ├── syntax  （增量 GFM 解析 + 语义树 + 定位查询 + 脏块重同步）
  │    → buffer, range
  └── history （undo/redo 事务栈，仅文本变更入栈，Reveal 不入栈）
       → buffer, annotate
```

职责一句话：buffer=存文本与 mark；range=带 bias 的跟随标记点；annotate=不修改源码的临时元信息；syntax=块级增量解析与双向定位；history=事务化撤销；session=唯一入口。

### B2 深模块判定

| 模块 | 调用方代码行数 | 深度 | 理由 |
|---|---|---|---|
| veil-core（Session facade） | ≤3 行/次交互 | 深 | `apply` 一次调用打包文本+注解变更，返回脏区事件；调用方不编排原语 |
| veil-ui（对 veil 而言） | ≤5 行/事件 | 深 | 事件翻译→`apply`→按 events 失效重绘，三步固定模式 |
| veil-core 内部模块 | 不公开 | — | crate 公开项 == 1（session 模块），远低于 ≤10 红线 |

浅模块检查：原设计中的 `offset_mapper`（并入 syntax 定位查询）、`Service Interface`（删除：渲染缓存句柄已走注解，内核无需异步接口）均不单列。

### B3 接缝

见 INSTANCE §7：接缝落点 `crates/veil/src/platform/`，覆盖窗口材质/系统主题/剪贴板/文件监听/IME 五项，上层零平台分支。机制选型：ADR-004（条件编译）。

### B4 依赖最小化

预算表见 INSTANCE §6。决策树执行记录：

| 决策点 | 需要吗 | 标准库能做吗 | 结论 |
|---|---|---|---|
| rope 存储 | 是（O(log N) 区间编辑是性能红线前提） | 否 | 引入，ADR-001 选型；手写 B-tree+delta 属基础设施，成本 > 依赖 |
| 增量 GFM 解析 | 是 | 否 | 手写：pulldown-cmark 为一次性解析且无 Range 绑定，不满足增量模型 |
| 异步运行时 | 否 | 是（std::thread + mpsc 覆盖后台渲染回调） | 不引入 tokio（黑名单） |
| grapheme 切分 | 是（光标移动正确性） | 否 | 引入 `unicode-segmentation` |
| JS 引擎（KaTeX） | 是 | 否 | `katex` crate（ADR-002） |

### B5 STRIDE（设计级）

| 数据流 | 威胁类别 | 具体场景 | 设计级缓解 |
|---|---|---|---|
| 打开 .md 文件 | DoS | 超深嵌套引用/标记使递归解析栈溢出；巨型文件耗尽内存 | 解析深度硬上限；块级增量解析天然摊销；文件大小软上限 + 可读错误 |
| 图片粘贴落盘 | Tampering | 构造 `../` 文件名路径穿越，写出文档目录 | 文件名 sanitize，锁定 assets 相对目录（veil 层） |
| HTML 导出 | Info Disclosure | raw HTML / `javascript:` 链接进入导出产物形成 XSS | 默认禁用 raw HTML；URL scheme 白名单 |
| 配置文件 | Tampering | 恶意/损坏 TOML 致启动崩溃或异常行为 | 解析失败回退默认配置 + 可读错误，不崩溃 |
| 外部文件变更 | Tampering | 监听通知与本地保存竞争，静默覆盖双方修改 | 内容哈希对比；冲突时写备份文件并提示，不静默覆盖 |

（Spoofing/Repudiation/Elevation：v1 无网络、无账户、无权限提升路径，过一遍后确认无实际威胁，不填。）

### B6 视觉令牌消费契约（S5/S7）

> 视觉令牌收敛到 `crates/veil-ui/src/tokens.rs` 的 8 个子模块与 `theme.rs` 的 `Theme` 结构体。
> 所有绘制/布局层（`paint.rs`、`chrome/paint.rs`、`paint_overlay.rs`、`view/*`）必须只从这些入口取值。
> 数值真源是代码；本文不复制任何数值，数值变更以 `tokens.rs` / `theme.rs` 为准。

#### 8 个唯一入口

| 模块 | 职责 | 消费方 |
|---|---|---|
| `tokens::surface` | 浮层/模态几何（圆角、行高、内边距、阴影层） | chrome、outline、quick_open、search、modal、settings |
| `tokens::palette` | 底色语义层（base / card / popup / modal / code / table_head / details） | 所有 paint_* |
| `tokens::state` | 交互态（hover / pressed / focus / disabled / selected / empty） | 所有 paint_* |
| `tokens::space` | chrome 间距刻度（4/8/12/16/24） | chrome、浮层布局 |
| `tokens::stroke` | 线宽（仅 hairline） | chrome 边框、浮层边框 |
| `tokens::typography` | 字阶与字体族 | 所有布局/绘制 |
| `tokens::motion` | 过渡时长与缓动 | 过渡（S6）、chrome 悬停淡入 |
| `tokens::doc` | 正文排版标尺（段距、列表缩进、代码块内边距、图片限高） | paint.rs、view/layout |

颜色值定义另在 `theme::Theme`（`bg`、`fg`、`chrome_bg`、`popup_bg`、`chrome_border`、`hover`、`thumb`、`accent`、`danger` 等字段），由 `palette` / `state` 模块经访问器对外暴露；绘制层不得直接读取 `Theme` 字段。

#### 关键依赖声明

- `surface::DIALOG_RADIUS > surface::CORNER_RADIUS`（模态比浮层大一级圆角，几何分离）
- `surface::HIGH_SHADOW_LAYERS` 层数 ≥ `SHADOW_LAYERS` 层数 + 1，且 alpha 浓于 Low 档（模态阴影更重）
- `surface::SCRIM_ALPHA_DARK > SCRIM_ALPHA_LIGHT`（深底遮罩需更重）
- `palette::surface_modal()` ≠ `palette::surface_popup()`（modal 底色比 popup 更突一层）
- `palette::surface_popup()` ≠ `palette::surface_card()`（popup 靠弹起，card 不弹）
- `typography::DIALOG_TITLE_SIZE > UI_SIZE` 且 `< BODY_SIZE`（模态标题层级居中）
- `tokens::space` 所有值必须是 4 的倍数；`tokens::doc` 不受此约束（正文排版节奏有独立历史）

#### 可覆盖 / 硬常量区分

- **可覆盖**：`Theme` 的颜色字段（支持浅/深两套 + 可配置扩展）、`SyntaxColors`（可从 `.tmTheme` 加载）、`scrim_alpha()` / `shadow_alpha()` 的 dark 增益
- **硬常量**：`surface` 模块所有几何（浮层形状是应用签名，不可随意改）、`space` / `stroke` / `typography` / `motion` / `doc` 模块、`palette` 的访问器（只暴露语义）
- `Theme::SyntaxColors` 的 plist 解析失败时回退硬编码兜底值（`SYNTAX_FALLBACK`）——这是语义安全兜底，不是漂移常量

#### 禁止条款（所有 paint_* / view/* 必须遵守）

1. **不得自行计算 corner_radius**：一律消费 `surface::corner_radius(scale)` 或 `surface::dialog_radius(scale)`
2. **不得直接访问 `theme.popup_bg` / `theme.chrome_bg`**：走 `palette::surface_popup(theme)` / `palette::surface_card(theme)`
3. **不得自行判断 hover/sel 状态色**：走 `state::hover_bg(theme)` / `state::pressed_bg(theme)`
4. **不得在绘制调用里内联 `[255, 255, 255, 255]` 等颜色字面量**（除 HTML `<span style>` 透传、纯测试夹具、模态徽标几何派生色）
5. **不得在绘制层内联间距数值**（如 `8.0 * scale`）：走 `space::S * scale` 或 `surface::*_padding(scale)`
6. **不得自行定义浮层阴影层**：浮层一律用 `SHADOW_LAYERS`，模态一律用 `HIGH_SHADOW_LAYERS`
7. **不得绕过 `scrim_alpha(dark)`** 直接写遮罩 alpha 硬编码（图片预览例外：预览遮罩语义独立，不是模态遮罩）
8. **不得新增不属于 4 倍数刻度的 chrome 间距**（正文排版 `doc::*` 不受此约束）

#### 例外（有理由保留的硬编码）

| 类别 | 位置 | 理由 |
|---|---|---|
| HTML `<span style>` 颜色透传 | `paint.rs::StyleBg` | 语义来自文档本身，必须透传 |
| 图片预览遮罩 | `image_preview.rs` | 视觉效果独立于模态遮罩 |
| 模态徽标感叹号几何 | `modal.rs::paint_error_icon` | 等比派生（`h*0.22`），非散落硬编码 |
| Details 浅主题底色 | `tokens::palette::details_card_bg` | palette 模块内部的语义值，同层无对外漂移 |
| chrome 顶层基准几何 | `chrome/mod.rs::MENU_H / STATUS_H / SCROLL_W` | 顶层基本量，不属于浮层/表面令牌家族 |
| 测试夹具里的 Frame 初始化底色 | 所有 `Frame::new(_, _, [255,255,255,255])` | 测试代码 |

#### 纪律执行

- `cargo test -p veil-ui` 下的 `dialog_tokens_are_single_source_and_consistent` / `palette_module_maps_theme_fields_correctly` / `space_tokens_are_multiples_of_four` / `high_shadow_layers_are_seven_layers_and_have_stronger_dark_gain` 共同锁定令牌一致性
- `docs/review/paint-hardcode-audit.md` 记录已知例外与待收敛项，每季度更新一次
