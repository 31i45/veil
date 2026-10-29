# G0 基线报告：B 战略 + A 质量门槛

> 评测日期：2026-10-26
> 对象：阶段 C 完成后的工作树（573 测试全绿）
> 计划：[阶段 04 执行计划](../04-strategy-execution-plan.md)
> 结论：**有条件 Go 进入阶段 0**；必须先处理超大块内存放大，再谈对外「快」的宣称

## 1. 结论摘要

| 维度 | 实测 | 判定 |
|---|---|---|
| 正常文档输入延迟（5005 块） | p95 **5.998ms** | 达标（≤16ms） |
| 1 万块输入延迟 | p95 **10.325ms** | 达标（≤16ms） |
| 冷启动到窗口就绪（10 万字/206KB） | 中位 **515.8ms**，最小 451.6ms | **未达 300ms 目标** |
| 冷首次编辑（5005 块） | **551.1ms**，其中 layout **543.3ms** | **阻断：首键可能冻结约半秒** |
| 常驻内存（10 万字文档） | 私有 **19.90MB**，工作集 39.78MB | 达标 |
| 产物体积 | **5,587,968B / 5.33MiB** | 达标 |
| 10MB 普通分段文档 | 55.5MB，响应正常 | 正常 |
| 10MB 单一超大块 | **19,235.6MB，Not Responding** | **P0 阻断：近似 OOM** |
| 病态输入崩溃 | 10 类用例 **0 panic**，源码往返一致 | 正面 |
| CommonMark 一致性 | A 66.1% / B 75.0% / C 69.2% | 未达标（P1） |
| 崩溃恢复/会话恢复 | 未实现 | **P0 阻断** |
| 跨编辑器同机对比 | Typora/VS Code/Obsidian 未安装 | **BLOCKED，不伪造数据** |

**一句话**：Veil 在「正常结构 + 正常规模」下很快；在「单个超大块」下会 OOM 级放大，在「冷首次编辑」上有半秒级尖峰，在「数据安全」上缺崩溃恢复。B 的「快」目前只对正常文档成立，不能对外做无条件宣称。

## 2. 环境与可复现性

| 项 | 值 |
|---|---|
| CPU | Intel Core i9-14900HX（24 核 / 32 线程） |
| 内存 | 31.7GB |
| OS | Windows 11 企业版 10.0.26300 |
| 构建 | `cargo build --release -p veil` |
| 二进制 | `target/release/veil.exe`，5,587,968B，SHA256 `9F461AFC…ADA8E` |
| 语料 | 约 10 万字符中文长文（206,270B）；另有 1/2/10MB 病态文件 |
| 测试 | `cargo test --workspace`：**573 passed / 0 failed / 5 ignored** |
| 静态门禁 | `cargo clippy --workspace --all-targets -- -D warnings`：**零警告** |

## 3. P2 性能基线

### 3.1 输入与布局

| 指标 | 结果 |
|---|---|
| `perf_keystroke_path`（5005 块，300 次） | p50 5.402ms，**p95 5.998ms**，max 6.367ms |
| `perf_keystroke_10k_blocks`（300 次） | p50 8.455ms，**p95 10.325ms**，max 14.356ms |
| `perf_cold_first_edit` | 总 **551.128ms**：build 0.080 / blocks 4.152 / editor 2.929 / **layout 543.347** / render 0.080 / paint 0.062 |
| `perf_peak_breakdown` | after_edit 峰值 **9.696ms**，paint 峰值 1.342ms |
| `bench_typing` 核心击键 | 178.30µs 中位 / 218.90µs p95 |
| 脏排版 | 173.90µs / 258.00µs p95 |
| 全量扫描（病态 12 次/键） | 38,693.80µs / 40,113.80µs p95 |
| 帧分配 vs 复用 | 1.68ms vs 0.28ms（复用约快 6 倍） |

**归因**：正常路径没有性能问题，p95 在 6–10ms。真正的问题是 **未预热的首编辑路径要全量布局 543ms**；这比后续任何一次击键都贵两个数量级。若用户在首帧呈现前输入，会感到明显停顿；需在 G1 用真实窗口复测该尖峰是否实际发生。

### 3.2 启动与资源

5 次冷启动（10 万字文档）：

| run | 到窗口就绪 | 私有内存 | 工作集 |
|---:|---:|---:|---:|
| 1 | 566.5ms | 19.90MB | 39.78MB |
| 2 | 451.6ms | 19.86MB | 39.75MB |
| 3 | 515.8ms | 19.90MB | 39.78MB |
| 4 | 567.2ms | 19.94MB | 39.80MB |
| 5 | 473.9ms | 19.96MB | 39.83MB |

- 中位 **515.8ms**，最小 451.6ms，最大 567.2ms；**未达 300ms 目标**。
- 内存中位 **19.90MB**，远低于 150MB 目标。
- 体积 **5.33MiB**，远低于 15MiB 目标。

### 3.3 跨编辑器对比

注册表与常见安装路径均未发现 Typora、VS Code、Obsidian；仅发现 MarkText 0.23.0（不在 P2 目标清单，未作为替代数据）。

**状态：BLOCKED。** 在安装并同机实测前，不得对外宣称「比 Typora 快」。

## 4. P3 鲁棒性冒烟

### 4.1 无 GUI 病态输入（10 类，0 panic）

| 用例 | 字节 | 耗时 | 块数 | 往返一致 |
|---|---:|---:|---:|---|
| empty | 0 | 0.02ms | 0 | 是 |
| CJK 8000 段 | 304,000 | 41.39ms | 8,000 | 是 |
| 10MB 单行 | 10,485,760 | 118.78ms | 1 | 是 |
| 未闭合围栏 10 万行 | 200,004 | 369.40ms | 1 | 是 |
| 2 万 `*` 嵌套 | 20,001 | 0.44ms | 1 | 是 |
| 5 千层 `[` 嵌套 | 10,004 | 0.10ms | 1 | 是 |
| 1 万列表项 | 118,890 | 46.80ms | 10,000 | 是 |
| 1 万行引用 | 40,000 | 18.36ms | 1 | 是 |
| 1000 行表格 | 10,024 | 0.82ms | 1 | 是 |
| 12 万块混合文档 | 937,780 | 7,069.70ms | 120,000 | 是 |

结论：**解析器本身没有 panic、没有挂起、往返保持一致**。10MB 单行解析只要 119ms，问题不在解析器。

### 4.2 GUI 超大块：P0 阻断

| 文件 | 内容形态 | 就绪 | 4–5s 后内存 | 响应 |
|---|---|---:|---:|---|
| 1MB 单行 | 一个巨型段落 | 738ms | **1,947.0MB** | 正常 |
| 2MB 单行 | 一个巨型段落 | 560.9ms | **3,878.7MB** | 正常 |
| 10MB 每 80 列换行 | 仍是一个巨型段落 | 586.6ms | **19,235.6MB** | **Not Responding** |
| 10MB 每段空行 | 13 万个小段落 | 695.4ms | 55.5MB | 正常 |
| 1MB 围栏代码 | 一个巨型代码块 | 678.6ms | **2,087.9MB** | 正常（4s 后） |

**根因定位**：块布局对整个块调用 `buffer.shape_until_scroll(..., false)`，一次性为块内全部字符生成字形；没有按可视行分片，也没有单块上限。[layout.rs](../../crates/veil-egui/src/view/layout.rs#L428-L431)

放大比约 **1.9GB / 1MB 文本**。在这台 31.7GB 机器上 10MB 单块已到 19GB；16GB 机器会直接触发系统级内存压力。**这比 panic 更危险**，因为用户可能丢失整个系统的工作。

### 4.3 数据安全

| 项 | 现状 | 证据 |
|---|---|---|
| 原子写 | tmp + rename，目标文件不会半截 | [main.rs](../../crates/veil/src/main.rs#L317-L335) |
| 保存失败保持 dirty | 已覆盖 | `save_failure_keeps_dirty` 等测试 |
| 备份失败不误报已保存 | 已覆盖 | `backup_failure_not_reported_as_saved` |
| 外部修改/删除提示 | 已覆盖 | `external_change_detected_before_overwrite`、`external_delete_marks_missing` |
| 崩溃恢复草稿 | **未实现** | 全仓无 recovery/draft 持久化 |
| 会话恢复/最近文件 | **未实现** | 全仓无 session restore/recent |
| 保存途中杀进程 | 原子写可保住目标文件；但内存中的未保存修改会丢 | 需阶段 0 故障注入验收 |

## 5. P4 专家启发式预审（基线切片）

静态截图 + 代码核对，不做用户盲测。

| # | 检查项 | 结果 |
|---|---|---|
| 1 | 首屏可输入、无空白/闪烁 | 通过（窗口就绪后立即可见） |
| 2 | 1 万字光标可见、滚动跟手 | 未验证（留 G1） |
| 3 | 中文 IME 候选跟随、组合期不错位 | 未验证（留 G1 真人） |
| 4 | 光标进出标记行高不跳 | 未验证（留 G1） |
| 5 | 块级视觉层级稳定 | 通过（标题/列表/引用/代码/表格可区分） |
| 6 | 查找替换可见、可取消 | 未验证（留 G1） |
| 7 | 菜单键盘可达、Esc 可退出 | 未验证（留 G1） |
| 8 | 外部修改/缺失提示 | 逻辑通过；UI 文案未验证 |
| 9 | 表格/图片可读可操作 | **部分**：表格可渲染，单元格不可编辑；图片格式受限 |
| 10 | 崩溃/退出不丢内容 | **不通过**：无崩溃恢复 |

### 5.1 截图发现：块级源码标记显隐不一致

- 标题始终显示 `#` / `##`，围栏代码始终显示 ` ```rust ` 与闭合 ` ``` `。
- 列表符号、引用 `>`、分隔线在非激活态会隐藏（`Sub` 占位）。
- 结果：同一篇文档里，有的块是 WYSIWYM，有的块是「源码 + 样式」。[layout.rs](../../crates/veil-egui/src/view/layout.rs#L737-L771)

这不是崩溃级问题，但与「一流编辑器」的视觉完成度冲突。需要产品决策：统一隐藏，或统一保留并明确为「源码真值」特征。建议阶段 1 处理，不在阶段 0 分心。

**已决策并落地（2026-10-29，阶段 1）**：统一为「非激活隐藏」——ATX 标题 `#`、Setext
下划线、围栏 ```（含语言标记）与列表/引用/分隔线同一规则；光标进入块（代码块含内容）
即还原源码。为满足「光标进出标记行高不跳」（G1 第 4 条），末行标记不能纯消隐（末尾纯空
行会丢行高），改用等长空格占位。实现见
[layout.rs](../../crates/veil-egui/src/view/layout.rs) `block_subs` 与三组布局断言。

## 6. 阻断项与优先级

| # | 阻断项 | 严重度 | 建议归属 |
|---|---|---|---|
| B1 | 超大单块内存放大（1.9GB/MB，10MB 单块 19GB 卡死） | **P0** | 阶段 0 第一片 |
| B2 | 冷首次编辑布局尖峰 543ms | **P0** | 与 B1 同根因，一起修 |
| B3 | 无崩溃恢复 / 会话恢复 / 最近文件 | **P0** | 阶段 0 |
| B4 | CommonMark 核心缺口（Setext/引用式链接/宽松列表/缩进代码） | **P0** | 阶段 0 |
| B5 | 启动 515.8ms 未达 300ms | P1 | 阶段 0 末/G1 |
| B6 | 跨编辑器「快」未验证 | P1 | 安装 Typora 后补测 |
| B7 | 标题/围栏源码标记显隐不一致 | P2 | 阶段 1 |
| B8 | README/INSTANCE 数字漂移（444/527 vs 573） | P2 | 阶段 0 统一数字来源 |

## 7. Go / No-Go

**结论：有条件 Go。**

1. **Go 进入阶段 0**：正确性与数据安全仍是主线，B 战略不变。
2. **条件一**：阶段 0 第一片必须先修 B1+B2（超大块布局保护 + 冷首编辑尖峰），否则一个 10MB 单块就可能让用户系统进入内存压力，谈不上「可信」。
3. **条件二**：在安装 Typora 并同机实测前，对外材料不得使用「比 Typora 快」；只能说「正常文档输入 p95 < 11ms，资源占用低」。
4. **条件三**：阶段 0 必须包含崩溃恢复，否则标准 1「不丢字」不成立。
5. **No-Go 项**：暂不进入阶段 1；表格编辑、图片、文件导航都排在 B1–B4 之后。

## 8. 阶段 0 建议顺序（待确认后执行）

| 切片 | 内容 | 退出条件 |
|---|---|---|
| 0a | 超大块布局保护：按可视行分片或单块上限；冷首编辑只布局可视块 | 1/2/10MB 单块不 OOM、可响应；冷首编辑 p95 ≤16ms 或明确降级提示 |
| 0b | 崩溃恢复 + 会话恢复 + 最近文件 | 任意时刻杀进程，重启可恢复到最后一次成功提交 |
| 0c | CommonMark 核心修复：引用式链接、Setext、宽松列表/懒续行、缩进代码 | P1 全量 A ≥85%，条件 C ≥90% |
| 0d | GFM 独立语料 + CI 门禁 | GFM ≥95%；每次提交自动跑 |

## 9. 复现命令

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cargo test -p veil-egui --release -- --ignored --nocapture
cargo run --release -p veil --example bench_typing

# 临时 P3 harness（不入库）
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'veil-g0\target'
cargo run --release --manifest-path (Join-Path $env:TEMP 'veil-g0\Cargo.toml')
```

原始数据：

- `%TEMP%\veil-g0-bench.txt`
- `%TEMP%\veil-g0-stress.txt`
- `%TEMP%\veil-g0-startup.csv`
- `%TEMP%\veil-g0-ui-1000x600.png`

## 10. 判定

- **B 战略成立，但「快」需要限定语**：正常结构快，超大单块不安全，冷首编辑有尖峰。
- **A 质量门槛未达标**，与 P1 结论一致。
- **本轮不修改产品代码**；G0 完成，停下等待确认后再进入阶段 0a。

## 11. 0a 验收复测（已完成）

> 对应计划：[04 执行计划 5.1](../04-strategy-execution-plan.md)。本节记录 0a 修复后的同机复测结果，替换 §3.1 与 §4.2 的阻断状态。

### 11.1 修复内容

| 项 | 实现 |
|---|---|
| 单块字节上限 | `MAX_BLOCK_BYTES = 64 KiB`：超限只排字符边界内的前缀 |
| 单块硬行上限 | `MAX_BLOCK_LINES = 512`：按整行回退，不切半个字形 |
| 光栅面积上限 | `MAX_RASTER_PX = 24,000,000`：几何高度与光栅同源截短，块尾不留白 |
| 显式降级 | `BlockLayout.truncated/full_len` + 块尾中文提示「后续内容未渲染；源码完整保留」 |
| 坐标钳位 | `caret_xy` 钳进可视区；`selection_rects` 跳过超区 run |
| 冷首编辑 | 沿用视口驱动惰性排版；首帧完成字体/字形预热，首键只碰可视块 |

### 11.2 超大单块内存（真实窗口，采样 5s 后）

| 文件 | 修复前 | 修复后 | 响应 |
|---|---:|---:|---|
| 1MB 单行 | 1,947.0MB | **129.5MB** | Responding=True |
| 2MB 单行 | 3,878.7MB | **131.8MB** | Responding=True |
| 10MB 单行 | 19,235.6MB | **149.1MB** | Responding=True（修复前 Not Responding） |
| 1MB 围栏代码 | 2,087.9MB | **100.2MB** | Responding=True |
| 10MB 普通分段 | 55.5MB | 55.6MB | 无变化（未走降级） |

降级前缀固定为 65,536 字节 / 15,708px 块高；10MB 单块滚轮 300 格后仍有 109,273 个内容像素，块尾提示可见（单测 `oversized_block_tail_is_painted_after_scroll` 实测 `notice_px=3999`）。

### 11.3 release 探针

| 探针 | 结果 |
|---|---|
| `perf_huge_block` 1MB | app 120.6ms / edit 101.3ms / 尾编辑 48.8ms，prefix 65,536，truncated |
| `perf_huge_block` 2MB | app 114.0ms / edit 103.8ms / 尾编辑 60.1ms |
| `perf_huge_block` 10MB | app 115.8ms / edit 187.3ms / 尾编辑 152.9ms |
| `perf_huge_scroll` 10MB | doc_h 15,740 / block_h 15,708 / scroll 15,196 / nonbg 109,273 |
| `perf_first_edit_after_first_frame` | app 105.4ms / 首帧 156.7ms / **首编辑 6.097ms** |
| `perf_keystroke_path`（5005 块热态） | p50 5.478ms / **p95 5.878ms** / max 6.201ms |

**冷首编辑归因修正**：G0 的 551.1ms 是在「App 构造后不做首帧、直接编辑」的合成路径上测得的（现测 326.4ms：app 134.2 + edit 192.2）。其中约 143–186ms 是字体面/字形的首次预热，真实窗口在用户可按键前必经首帧（156.7ms）并已支付该成本；首帧之后的首键为 **6.097ms**，热态 p95 5.878ms。因此：

- **用户可感知的冷首编辑满足 p95 ≤16ms**（首键 6.1ms）。
- 「App 构造后零帧直接编辑」这一合成路径仍为百毫秒级，作为明确降级记录：只布局可视块（19/5005），不 OOM、可响应，且真实事件循环不会先于首帧派发按键。

### 11.4 回归门禁

| 项 | 结果 |
|---|---|
| `cargo test --workspace` | **578 passed / 0 failed / 9 ignored**（基线 573 passed；新增 5 个回归测试 + 4 个 ignored 探针） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| 源码真值 | 超大块文首/文末编辑后 `serialize()` 完整往返（含降级块） |
| 降级几何一致性 | 光栅高度 ≤ 块高（单测 `oversize_area_cap_keeps_height_and_raster_in_sync`） |

### 11.5 结论

- **B1 关闭**：1/2/10MB 单块全部不 OOM、可响应；10MB 普通分段资源占用不变。
- **B2 关闭（带限定语）**：真实首键 6.1ms，热态 p95 5.9ms；合成零帧路径为明确降级（只排可视块 + 前缀上限），不构成用户路径。
- **未改语义**：超限块的源码完整保留，只是不渲染；提示文案明确说明「未渲染」。
- **0a 完成**，等待确认后进入 0b（崩溃/会话恢复）。

## 12. 0b 验收记录（已完成）

> 对应计划：[04 执行计划 5.2](../04-strategy-execution-plan.md)。本节替换 §1 中「崩溃恢复/会话恢复：未实现（P0 阻断）」的判定。

### 12.1 实现内容

| 项 | 实现 |
|---|---|
| 恢复草稿格式 | `document/recovery.rs`：魔数头 + BOM 标记 + 路径 + 全文，纯函数编解码；FNV-64 内容哈希 |
| 落盘时机 | 编辑静默 2s（复用 Autosave deadline）；草稿无路径/冲突挂起也写；IME 组合期不写 |
| 落盘位置 | `%APPDATA%\Veil\recovery.dat` 单槽，产品层 tmp+rename 原子写 |
| 防重复 | 内容哈希相同不重写；写失败记时刻按 5s 退避重试 + 状态栏 notice，绝不 panic |
| 清理 | 保存/另存为成功后清除；清理失败残留由启动时「恢复内容 == 磁盘内容」判定自清 |
| 启动决议 | `startup_doc`：崩溃恢复 > 会话恢复 > 传入文档；仅无 CLI 参数（`Doc.resume`）启用 |
| 恢复语义 | 恢复内容标脏，基线 = 磁盘当前内容（与「无崩溃」编辑-保存路径等价）；磁盘读不到置缺失态 |
| 损坏容错 | 解码失败 → eprintln + 清除 + 正常打开 |
| 会话/最近文件 | `document/session.rs`：`session.txt` 行格式（容错解析），上限 10 条去重提头；打开/另存为/启动成功登记 |
| 菜单动态化 | `chrome::menu_rows` 为布局/命中/分发的单一来源；最近文件插在「退出」之前，上方分隔线；`MenuAction::OpenRecent(i)` → `open_path` |

### 12.2 故障注入测试（headless，新增 25 个）

| 类别 | 测试 | 证明 |
|---|---|---|
| 杀进程等价路径 | `recovery_written_after_idle_and_restored_on_restart` | 编辑→2s 落快照→重启恢复为脏文档，基线=磁盘 |
| 保存清理 | `recovery_cleared_after_successful_save` | 保存成功即清恢复副本 |
| 残留自清 | `recovery_matching_disk_opens_clean_and_self_clears` | 恢复==磁盘 → 干净打开 + 自清 |
| 草稿恢复 | `recovery_draft_without_path_restored_dirty` | 无路径草稿照恢复且标脏 |
| 磁盘缺失 | `recovery_missing_disk_marks_missing` | 恢复照给 + 缺失提示态 |
| 写失败 | `recovery_write_failure_retries_with_backoff` | 不 panic + notice + 5s 退避 + 故障恢复后落盘 |
| 去重 | `recovery_same_content_not_rewritten` | 哈希相同不重写，变更才重写 |
| IME | `recovery_skipped_during_ime_preedit` | 组合期不写快照 |
| 损坏 | `corrupted_recovery_ignored_and_cleared` | 忽略 + 清除 + 正常打开 |
| 外部修改共存 | `recovery_with_external_disk_change_saves_with_backup` | 恢复后外部再改 → 保存走冲突备份 |
| CLI 隔离 | `cli_open_leaves_recovery_untouched` | CLI 打开不读不动恢复槽 |
| 会话恢复 | `session_resume_opens_recent_file` / `session_resume_failure_falls_back_to_sample` | 打开最近文件；失败回退样例 |
| 最近列表 | `open_paths_update_recent_in_order_and_persist` + `document/session.rs` 3 个单测 | 去重/提头/上限/持久化 |
| 动态菜单 | `file_menu_injects_recent_before_quit` + `menu_click_opens_recent_file` + `open_recent_ignored_when_dirty_or_out_of_range` | 布局/命中/分发同源；端到端点击打开；脏拒绝/越界忽略 |
| 格式编解码 | `document/recovery.rs` 5 个单测 | 往返/中文/CRLF/草稿/损坏拒绝/哈希稳定 |

### 12.3 真实进程冒烟（杀进程）

`veil.exe hello.md`（隔离 APPDATA）→ 3s 后 `Stop-Process -Force`：
- `Veil\session.txt` 含 `recent=<hello.md 全路径>`（原子写完整，无半截）。
- `recovery.dat` 不存在（无编辑即无快照，行为正确）。
- 恢复写盘路径需真实按键，本环境无 GUI 自动化，由 §12.2 的 tick 级故障注入覆盖（同一 `maybe_recovery_write` 代码路径）。

### 12.4 回归门禁

| 项 | 结果 |
|---|---|
| `cargo test --workspace` | **603 passed / 0 failed / 9 ignored**（0a 基线 578；0b 新增 25） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |

备注：`memo_hit_equals_forced_after_any_sequence` 在一次并行全量运行中超时失败一次，单独与全量复跑均通过（90s 级穷举测试，对并行负载敏感），与 0b 改动无因果关系。

### 12.5 结论

- **P0「崩溃恢复/会话恢复」关闭**：任意时刻杀进程，重启后可恢复到最后一次成功提交的恢复快照（2s 静默粒度）；「不丢字」支柱补齐最后防线。
- **0b 完成**，等待确认后进入 0c（CommonMark 正确性）。
