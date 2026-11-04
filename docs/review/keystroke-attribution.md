# 击键分项归因基线（三档文档规模）

> 日期：2026-11-03 · 机器：本机 Windows（`--release`） · 样本：每档 300 次单字符插入
> 探针：`crates/veil-ui/src/app_tests.rs::perf_keystroke_attribution`（`#[ignore]`，不进常规 `cargo test` 门禁）
> 运行命令：
>
> ```powershell
> cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_keystroke_attribution
> ```

## 结论

**索引层占合计 p95 的 65.5% / 72.9% / 77.0%，三档均远超 20% 触发表决阈值——应改索引。**
其中 `index_rebuild` 段是生产代码 `App::refresh_index` 的全段真实耗时，已包含块清单枚举、
估算高度、`recompute_geometry` 与 `refresh_stats`（全文字数重算）；本探针不再细分该段内部，
以免超出任务约定的 5 段口径。

## 1. 段定义与可测性

| 段 | 测量的真实路径 | 是否真实 |
|---|---|---|
| `core_apply_reparse` | `Editor::apply(Intent::InsertText)` → `Session::apply`（含增量重解析） | 真实（含极薄的 Editor 意图分发） |
| `index_rebuild` | `App::refresh_index`（块枚举 + 估高 + `recompute_geometry` + `refresh_stats`） | 真实 |
| `dirty_layout` | 对 `Events::dirty_blocks` 逐块 `App::ensure_block`（缓存键查询 + 取词/行内 + `layout_block`） | 真实 |
| `rasterize` | 脏块 `LayoutEngine::rasterize` + 回填光栅缓存（与 `paint.rs` 惰性光栅同一路径） | 真实 |
| `frame_compose` | `App::render`（headless 帧合成；`present` 无 surface 时 no-op，不计时） | 真实（不含上屏拷贝） |

说明：`frame_compose` 不需要真实窗口——`render` 在 headless 下走完整布局/光栅/绘制路径；
未测的只有 `present` 的 surface 拷贝（OS 侧开销，与本次“改不改索引”的决策无关）。
本文所有数字均为本次真实运行输出，无估算。

## 2. 三档实测（单位：µs）

文档由 `stress_doc_n(iters)` 构造（每轮 7 块）：715 轮 → 5005 块；1500 轮 → 10500 块；
2857 轮 → 19999 块。

| 文档规模（块） | 段 | p50 | p95 | 占合计 p95 |
|---|---|---|---|---|
| 5005 | `core_apply_reparse` | 764.70 | 958.20 | 12.07% |
| 5005 | `index_rebuild` | 4656.50 | **5203.10** | **65.54%** |
| 5005 | `dirty_layout` | 120.70 | 190.20 | 2.40% |
| 5005 | `rasterize` | 161.40 | 302.10 | 3.81% |
| 5005 | `frame_compose` | 1024.00 | 1285.30 | 16.19% |
| 5005 | **合计 p95** | — | **7938.90** | 100% |
| 10500 | `core_apply_reparse` | 1558.20 | 1881.70 | 13.60% |
| 10500 | `index_rebuild` | 9427.20 | **10089.20** | **72.94%** |
| 10500 | `dirty_layout` | 126.40 | 185.10 | 1.34% |
| 10500 | `rasterize` | 125.40 | 254.00 | 1.84% |
| 10500 | `frame_compose` | 1178.50 | 1423.00 | 10.29% |
| 10500 | **合计 p95** | — | **13833.00** | 100% |
| 19999 | `core_apply_reparse` | 3186.40 | 3571.00 | 14.61% |
| 19999 | `index_rebuild` | 17771.70 | **18812.70** | **76.96%** |
| 19999 | `dirty_layout` | 127.80 | 165.70 | 0.68% |
| 19999 | `rasterize` | 119.40 | 200.30 | 0.82% |
| 19999 | `frame_compose` | 1426.80 | 1693.90 | 6.93% |
| 19999 | **合计 p95** | — | **24443.60** | 100% |

同一探针的另一轮运行（用于校验稳定性）：`index_rebuild` p95 = 5029.1 / 10249.5 / 18256.7µs，
占比 65.5% / 73.2% / 76.8%，结论与上表一致。

“合计 p95” = 五段 p95 之和（每段独立统计，不是同一笔击键的端到端 p95）；百分比按该合计计算。

## 3. 读法

1. **索引层是唯一主项**：20k 块时 `index_rebuild` p95 18.81ms，占 77.0%；脏块排版与光栅合计
   不足 0.4ms，说明“变慢”不在 cosmic-text 排版，也不在光栅。
2. **随规模近似线性放大**：5005→19999 块，索引 p95 5.20→18.81ms（约 3.6×），与块数增长
   （约 4×）同量级；脏块段基本不随规模变化（190→166µs），符合“每次只动 1 块”的增量设计。
3. **`core_apply_reparse` 也随规模增长**（0.96→3.57ms），属内核 `blocks`/行号换算与解析结构的
   线性成本；但单个击键的增量重解析本身未主导。
4. **决策**：`index_rebuild` 已远超 20% 阈值，优先改造视图侧索引重建（块清单枚举、几何累计、
   全文字数统计的增量化/惰性化）；同时 `core` 段的线性成本应与索引改造一并复核，避免只优化
   一半路径。

## 4. 复现与门禁

- 探针为 `#[ignore]` 测试，`cargo test --workspace` 不执行它，仅编译，不拖慢常规门禁。
- 代码位置：`crates/veil-ui/src/app_tests.rs`（`stress_doc_n` + `perf_keystroke_attribution` +
  `perf_index_subsegments`）；索引拆分实现位于 `crates/veil-ui/src/document/block_index.rs`。
- 为保证机器状态可比，运行前自建 5005/10500/19999 块文档，每档预热一次 `paint()` 后再采样。
- 测量时工作树含其他并行改动（不属于本探针）；本文基线对应当前工作树状态，后续合入会改变
  绝对值，应重新运行探针刷新。

## 5. `index_rebuild` 子段拆分（追加运行）

> 日期：2026-11-03 · `--release` · 每档 300 样本
> 命令：`cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_index_subsegments`
> 四步按生产 `refresh_index` 的同一顺序分别调用 `App::blocks_enum` → `App::geom_fill` →
> `App::recompute_geometry` → `App::refresh_stats`；计时全部在探针外部完成，生产热路径无计时钩子。

| 文档规模（块） | 子段 | p50 (µs) | p95 (µs) | 占 index 合计 p95 |
|---|---|---|---|---|
| 5005 | `blocks_enum` | 3183.20 | **3381.00** | **70.0%** |
| 5005 | `geom_fill` | 828.60 | 925.80 | 19.2% |
| 5005 | `recompute` | 139.00 | 158.60 | 3.3% |
| 5005 | `stats` | 314.40 | 363.80 | 7.5% |
| 5005 | **index 合计** | 4465.20 | **4829.20** | 100% |
| 10500 | `blocks_enum` | 6591.70 | **6941.50** | **68.3%** |
| 10500 | `geom_fill` | 1811.60 | 2044.70 | 20.1% |
| 10500 | `recompute` | 275.70 | 342.70 | 3.4% |
| 10500 | `stats` | 672.90 | 839.90 | 8.3% |
| 10500 | **index 合计** | 9351.90 | **10168.80** | 100% |
| 19999 | `blocks_enum` | 12521.30 | **13150.00** | **69.1%** |
| 19999 | `geom_fill` | 3490.20 | 3808.10 | 20.0% |
| 19999 | `recompute` | 526.00 | 632.50 | 3.3% |
| 19999 | `stats` | 1194.00 | 1440.00 | 7.6% |
| 19999 | **index 合计** | 17731.50 | **19030.60** | 100% |

说明：本节与主表来自不同运行轮次，index 数值存在 0.8%~7% 的波动；子段占比在同一轮内计算，
用于归因排序。**第一成本是 `blocks_enum`（占 index p95 的 68.3%~70.0%），其次 `geom_fill`
（19.2%~20.1%）；`recompute` 与 `stats` 合计仅约 11%~12%。** 因此改造应优先针对每次击键的
全量块枚举（`session.blocks()` 的整表重建与 line→byte 换算），而不是几何累计。

### 生产无残留计时的保证

`refresh_index` 被拆成四个普通 `pub(crate)` 方法，生产实现仅按原顺序调用它们，方法体内没有
`Instant::now()`、条件编译分支或日志；`perf_index_subsegments` 位于 `#[cfg(test)]` 的
`app_tests` 模块，只在测试构建中编译，release 正常运行的生产目标不会包含该探针代码。

## 6. 2026-11-03 最终复测

> 日期：2026-11-03 · release 构建 · headless 探针 · 每档 300 次单字符插入
> 命令：`cargo test -p veil-ui --release --lib perf_keystroke_attribution -- --ignored --nocapture`

| 块数 | core p95 | index p95 | layout p95 | raster p95 | compose p95 | 合计 p95 | index 占比 |
|---|---|---|---|---|---|---|---|
| 5.0k | 966 µs | 951 µs | 121 µs | 302 µs | 1027 µs | 3367 µs | 28.2% |
| 10.5k | 1847 µs | 1769 µs | 129 µs | 223 µs | 1235 µs | 5203 µs | 34.0% |
| 20.0k | 3887 µs | 3307 µs | 150 µs | 292 µs | 1613 µs | 9248 µs | 35.8% |

改动前后对比（同一探针、同机、release）：

| 块数 | 改动前合计 p95 | 改动后合计 p95 | 降幅 | index p95 改动前 → 改动后 |
|---|---|---|---|---|
| 5.0k | 7939 µs | 3367 µs | −57.6% | 5203.10 → 951 µs |
| 10.5k | 13833 µs | 5203 µs | −62.4% | 10089.20 → 1769 µs |
| 20.0k | 24444 µs | 9248 µs | −62.2% | 18812.70 → 3307 µs |

关键改动：

- `Session::blocks()` 改用 `Block` 持有的字节 `Range`，`blocks_enum` p95 6941 → 196 µs（10.5k）；
- `refresh_index` 去掉与 `recompute_geometry` 重复的布局探测，并对同 id / 同字节长度的块复用 `est_h`。

读法：§2/§5 旧基线的 index 占比 65.5%–77.0% 已随上述改造失效，本节为当前真值。index 仍占合计
p95 的 28.2%–35.8%，高于 20% 触发表决阈值，但其绝对成本已从 5.2/10.1/18.8 ms 降到
0.95/1.77/3.31 ms。合计 p95 同样按五段独立 p95 求和，未包含 `present` 的 surface 拷贝。
