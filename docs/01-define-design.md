# Skill 01 产出 · 定义 + 设计（Veil）

> 有效期：需求或架构变更时需更新
> 最后验证：2026-10-24（定义完成门通过；本文为 v1 设计基线）
> 引用约定：数字红线、依赖预算、接缝清单、输入点清单一律以 [.skills/INSTANCE.md](../.skills/INSTANCE.md) 为唯一来源，本文不复制数值，只引用条款。

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
| egui IME 输入抽象不支持自绘 preedit | ✅ 已验证（2026-10-24 spike，`spikes/winit-ime`，winit 0.30 + 微软拼音真机人工确认）：`Ime::Preedit` 逐击键送达（组合串 + 光标字节区间），`Ime::Commit` 文本正确，`set_ime_cursor_area` 候选框跟随正确 | 风险解除：绕开 egui 输入抽象、在 winit 层直收 IME 事件的路径成立（INSTANCE §7 接缝不变） |
| JS 引擎跑 KaTeX 首屏性能不足（公式密集文档） | ✅ 已验证（2026-10-24 spike，`spikes/katex-timing`）：`katex` crate 在 windows-msvc 不可用（quick-js 后端不支持该 target；duktape 后端缺 `Array.prototype.fill`，矩阵环境渲染失败），主路径改为 `rquickjs` 直集成（ADR-002 已修订）。实测：引擎冷启动 29ms，100 公式 210ms（均值 2.1ms/个） | 100 公式同步渲染占 70% 冷启动预算 → 异步渲染 + `BlockRenderCacheId` 缓存确认为必要设计，首屏禁止同步阻塞 |

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
