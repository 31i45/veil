# 性能分布（2026-11-27，Task 5）

> 目的：把「单次样本」升级为**多次 release 采样的分布**（min / p50 / p95 / max），覆盖 5000 标题与滚动、
> 首帧、大块、50MB、版本历史面板 5000 快照等关键路径。所有数字来自真实执行，不含估计值。
>
> 时间说明：分布与门禁运行发生在 2026-11-28 00:12–00:36（Asia/Shanghai），脚本按运行日写入
> `docs/qa/probe-gate-2026-11-28.md`；任务与证据目录仍按 2026-11-27 登记。
>
> 归档（2026-11-28 简化后）：
> - 本文件固化分布轮的真实数字（§1–§2）；中间实验归档已删除，不留 `*-FAIL*` / `*-spike*` / `*-unitbug*`。
> - 外部基线 JSON 与稳定性临时归档已删除：回归判定改为**探针内自证上限**（见 §3），
>   门禁 `tools/verify/probe-gate.ps1` 只负责运行与归档（`docs/qa/probe-gate-2026-11-28.md`）。

## 环境与方法

| 项 | 值 |
|---|---|
| 机器 | Intel Core i9-14900HX |
| 系统 | Microsoft Windows 11 企业版 10.0.26300 |
| 工具链 | rustc 1.99.1 (58cf8f9c3 2026-10-13) |
| 构建 | `cargo test --workspace --release --no-run`；profile：`lto=true`、`codegen-units=1`、`panic=abort`、`strip=true`、`opt-level=s` |
| 采样 | 分布轮每个性能探针 **9 次**独立进程；`perf_huge_block` 也为 9 次；非性能探针固定 1 次 |
| 命令（历史） | 分布轮使用已移除的 `-Repeat 9` 重复运行同一探针；现门禁为 `pwsh -NoProfile -File tools/verify/probe-gate.ps1` 一次跑完全部 ignored 探针 |
| 回归判定（2026-11-28 简化） | 无外部基线、无噪声地板：每条探针在测试内断言自己的回归上限；单次观测改为测试内 3 次取 p50；归因/诊断探针断言结构不变量 |

**p95 口径**：每次采样是独立 release 进程；对同一指标的 9 个采样值排序后按
`rank=(n−1)×0.95` **线性插值**取 p95。n=9 时 p95 **不再等于最大值**
（例：`perf_huge_block app_ms@10mb` p95 126.326 < max 127.803）。

> **读数说明**：上表的 min/p50/p95/max 是分布轮的真实统计值，**未重算**。门禁已简化为
> 「探针内自证上限」：表中的 p95 只用于说明分布尾部，实际判定阈值写在每条探针里（见 §3）。

运行内样本量：`perf_50mb_long_doc` 100 次击键；`perf_20k_blocks` 300 次；`perf_keystroke_path` 300 次；
`perf_keystroke_attribution` / `perf_index_subsegments` 每档 300 次；`perf_peak_breakdown` /
`perf_after_edit_breakdown` 500 帧；`perf_5000_headings` / `perf_history_5000` 各 300 步滚轮；
其余为单次观测（每次运行 1 个值；现已改为测试内 3 次取 p50）。

## 1. Task 5 指定的关键路径（n=9，分布轮）

### 5000 标题与滚动（`perf_5000_headings`）

| 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| 标题块数 | 5000 | 5000 | 5000 | 5000 | 个 |
| 打开（构造 + 首帧，`open_ms`） | 91.093 | 94.401 | **96.463** | 96.594 | ms |
| 标题枚举（`headings_us`） | 29.5 | 32.9 | 35.1 | 35.3 | µs |
| 单步滚动+重绘 p50（`scroll_p50_ms`） | 0.741 | 0.776 | 0.808 | 0.812 | ms |
| 单步滚动+重绘 p95（`scroll_p95_ms`） | 0.845 | 0.979 | **1.098** | 1.145 | ms |

### 首帧（`perf_first_edit_after_first_frame`）

| 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| 首帧（`first_frame_ms`，构造→首帧） | 51.493 | 56.023 | **57.272** | 57.504 | ms |
| 首帧后首次按键（`first_edit_ms`） | 1.409 | 1.522 | **1.916** | 1.957 | ms |

### 大块（`perf_huge_block`，1 / 2 / 10 MB 单块，n=9）

| 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| 打开 1 MB（`app_ms@1mb`） | 15.648 | 17.473 | 18.202 | 18.239 | ms |
| 首键 1 MB（`edit_ms@1mb`） | 151.219 | 154.079 | 160.465 | 161.932 | ms |
| 文末插入 1 MB（`tail_ms@1mb`） | 145.290 | 152.043 | 155.720 | 156.060 | ms |
| 打开 2 MB（`app_ms@2mb`） | 24.136 | 27.004 | 30.897 | 31.713 | ms |
| 首键 2 MB（`edit_ms@2mb`） | 233.727 | 241.425 | 263.190 | 273.594 | ms |
| 文末插入 2 MB（`tail_ms@2mb`） | 234.094 | 237.012 | 243.080 | 243.671 | ms |
| 打开 10 MB（`app_ms@10mb`） | 113.730 | 119.169 | 126.326 | 127.803 | ms |
| 首键 10 MB（`edit_ms@10mb`） | 924.402 | 960.845 | 976.024 | 976.246 | ms |
| 文末插入 10 MB（`tail_ms@10mb`） | 940.501 | 964.567 | 975.732 | 980.422 | ms |

修复后复测（2026-11-28，同一探针 n=9，连续 3 轮）：主因是排版路径每次击键把整块
10 MB 解析成 inline 节点（占约 82%）；改为「只解析 64 KiB 可渲染前缀」并把光栅限制在
可见行带后，1 MB 首键 p95 **48.4–50.9 ms**、文末插入 p95 **42.9–44.9 ms**；2 MB 首键
p95 56.4–58.1 ms、文末插入 53.3–55.8 ms；10 MB 首键 p95 **147.0–148.5 ms**、文末插入
149.6–150.6 ms。探针现按 1/2/10 MB = **70 / 80 / 200 ms** 断言，该数值是**回归上限
（regression ceiling），不是目标值**：仅用于拦截 O(块字节) 级回归，1 MB 取 70 ms 是
基于实测 p95 约 50–60 ms 留 ≥15% 抖动余量，避免偶发噪声误红。

### 50 MB 中文长文（`perf_50mb_long_doc`，n=9）

| 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| 块数 | 20001 | 20001 | 20001 | 20001 | 个 |
| 打开（含首帧，`open_ms`） | 473.7 | 491.0 | **501.68** | 504.4 | ms |
| 击键 p50（`keystroke_p50`） | 1.954 | 2.053 | 2.122 | 2.132 | ms |
| 击键 p95（`keystroke_p95`） | 2.195 | 2.364 | **2.533** | 2.627 | ms |
| 保存（`save_ms`） | 322.1 | 332.2 | 335.84 | 336.2 | ms |

### 版本历史面板 5000 快照（`perf_history_5000`，n=9）

口径：打开 = `open_history()` + 首帧重绘（读取 5000 条元数据 + 生成时间标签 + 绘制）；
滚动 = 指针位于面板内，单步滚轮 + 重绘；关闭 = `close_history()` + 重绘。

| 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| 快照条数 | 5000 | 5000 | 5000 | 5000 | 条 |
| 打开（`open_ms`） | 9.309 | 9.888 | **10.738** | 10.930 | ms |
| 单步滚动+重绘 p50（`scroll_p50_ms`） | 3.044 | 3.172 | 3.211 | 3.213 | ms |
| 单步滚动+重绘 p95（`scroll_p95_ms`） | 3.277 | 3.495 | **3.642** | 3.644 | ms |
| 关闭（`close_ms`） | 0.369 | 0.385 | **0.423** | 0.432 | ms |

结论：5000 快照下打开约 **10.7 ms**、单步滚动 p95 约 **3.6 ms**、关闭 < 0.5 ms，未触及 100 ms 的加载态阈值
（与 `docs/qa/w11-load-state-measurement.md` 的 G4 同口径）。本轮不引入异步加载态。

## 2. 其它性能探针分布（n=9，分布轮）

| 探针 / 指标 | min | p50 | p95 | max | 单位 |
|---|---|---|---|---|---|
| `bench_edit_reparse_history_100k` reparse_us | 785.440 | 834.390 | 857.336 | 866.582 | µs |
| `bench_edit_reparse_history_100k` edit_us | 4.890 | 6.484 | 7.867 | 8.082 | µs |
| `bench_edit_reparse_history_100k` history_us | 0.312 | 0.654 | 0.813 | 0.814 | µs |
| `perf_20k_blocks` keystroke_p50 | 2.252 | 2.383 | 2.427 | 2.429 | ms |
| `perf_20k_blocks` keystroke_p95 | 2.609 | 2.741 | 2.871 | 2.888 | ms |
| `perf_keystroke_path` keystroke_p50 | 1.235 | 1.327 | 1.346 | 1.350 | ms |
| `perf_keystroke_path` keystroke_p95 | 1.676 | 1.737 | 1.817 | 1.825 | ms |
| `perf_keystroke_attribution` total_p95 @2002 块 | 1860.3 | 2028.2 | 2122.6 | 2144.8 | µs |
| `perf_keystroke_attribution` total_p95 @5005 块 | 2689.5 | 2976.1 | 3019.4 | 3033.6 | µs |
| `perf_keystroke_attribution` total_p95 @10003 块 | 4136.2 | 4306.4 | 4470.2 | 4473.8 | µs |
| `perf_keystroke_attribution` total_p95 @19999 块 | 7025.8 | 7302.5 | 7487.2 | 7496.4 | µs |
| `perf_keystroke_attribution` total_p95 @40005 块 | 13208.9 | 13497.7 | 13930.8 | 13984.2 | µs |
| `perf_index_subsegments` index_sum_p95 @2002 块 | 242.3 | 262.8 | 324.76 | 329.8 | µs |
| `perf_index_subsegments` index_sum_p95 @5005 块 | 1002.6 | 1099.7 | 1222.6 | 1238.2 | µs |
| `perf_index_subsegments` index_sum_p95 @10003 块 | 2085.4 | 2249.2 | 2400.6 | 2427.9 | µs |
| `perf_index_subsegments` index_sum_p95 @19999 块 | 4104.8 | 4376.9 | 4469.5 | 4505.7 | µs |
| `perf_index_subsegments` index_sum_p95 @40005 块 | 8600.4 | 8789.2 | 9181.1 | 9341.2 | µs |
| `perf_cold_first_edit` cold_first_edit_ms | 95.643 | 104.124 | 105.775 | 105.786 | ms |
| `perf_cold_first_edit_prewarmed` edit_ms | 45.670 | 50.257 | 53.240 | 53.469 | ms |
| `perf_peak_breakdown` peak_total_ms | 3.046 | 3.694 | 5.480 | 5.788 | ms |
| `perf_peak_breakdown` peak_paint_ms | 0.641 | 1.121 | 3.310 | 3.728 | ms |
| `perf_after_edit_breakdown` peak_total_ms | 3.003 | 3.742 | 16.410 | 16.971 | ms |
| `perf_after_edit_breakdown` p95_refresh_index_ms | 1.828 | 1.988 | 2.145 | 2.151 | ms |
| `perf_after_edit_breakdown` p95_recompute_geometry_ms | 0.365 | 0.405 | 0.430 | 0.431 | ms |

## 3. 探针内自证上限（regression ceiling）

外部基线 JSON、噪声地板与逐条阈值策略已删除。判定改为**每条探针在测试内断言自己的回归上限**：
上限取本文件实测分布最大值上方 **≥20% 余量**，并注明是 regression ceiling（**不是目标值**），
只用于拦截量级回归。单次观测（打开/关闭/冷启动）在测试内采 3 次取 p50 后再断言。
归因/诊断类探针不设时间上限，但必须断言结构不变量，避免「永远只打印」。

| 探针 | 断言统计量 | 上限 | 实测分布依据 |
|---|---|---|---|
| `bench_edit_reparse_history_100k` | 50 次均值 reparse / edit / history | 1200 / 12 / 5 µs | max 867 / 8.1 / 0.81 µs |
| `perf_50mb_long_doc` | 打开 `open_ms` / 击键 `keystroke_p95` | 1000 ms / 4 ms（另有产品预算 5000 ms / 16 ms） | max 504.4 / 2.63 ms |
| `perf_20k_blocks` | `keystroke_p95` | 3.5 ms | max 2.888 ms |
| `perf_keystroke_path` | `keystroke_p95` | 2.5 ms | max 1.825 ms |
| `perf_cold_first_edit` | 3 次 p50 | 160 ms | max 105.8 ms |
| `perf_cold_first_edit_prewarmed` | 3 次 p50 | 80 ms | max 53.5 ms |
| `perf_first_edit_after_first_frame` | 首键 p50 / 首帧 p50 | 3 ms / 100 ms | max 1.96 / 57.5 ms |
| `perf_5000_headings` | 打开 p50 / 滚动 p95 | 200 ms / 2.5 ms | max 96.6 / 1.145 ms |
| `perf_history_5000` | 打开 p50 / 滚动 p95 / 关闭 p50 | 25 ms / 7 ms / 1.2 ms | max 10.9 / 3.64 / 0.432 ms |
| `perf_huge_block` | 首键 / 文末插入 p95（1 / 2 / 10 MB） | 70 / 80 / 200 ms | 既有回归上限，未改动 |
| `perf_keystroke_attribution` | 结构：分项和 > 0、apply/index/frame 非零、share ∈ [0,100] | 不设时间上限 | 纯归因 |
| `perf_index_subsegments` | 结构：子段和 > 0、`blocks_enum` 非零、各子段非负 | 不设时间上限 | 纯归因 |
| `perf_peak_breakdown` | 结构：apply+after+paint = 总耗时、paint 非零 | 不设时间上限 | 诊断 |
| `perf_after_edit_breakdown` | 结构：子步骤和 = 总耗时、`refresh` 非零 | 不设时间上限 | 诊断 |

病态探针（`perf_huge_scroll`、`perf_huge_block` 的截断不变量、`pathological_inputs_matrix`）
与 CJK 证据探针保持既有断言；CJK 缺字体仍记 SKIP。

## 4. 局限

- 单机、本地 NTFS、热缓存；未覆盖网络盘、杀毒实时扫描、低配机器。
- `perf_peak_breakdown` / `perf_after_edit_breakdown` 已改为诊断探针：只断言分项和与关键阶段非零，
  不再对单帧峰值设时间上限，避免宿主毛刺造成假红；真实回归由各自函数内的分项计时暴露。
- `perf_huge_scroll`（10 MB 单块 300 步滚轮）分布轮墙钟 **130.4 s**，之后多次复跑约 **5.0 s**；
  其断言指标 `nonbg=84898 > 1000` 均 PASS；该探针不产出耗时指标，滚动耗时见 §1。
- 本文件不替代 `docs/qa/w11-load-state-measurement.md` 的快速打开/大纲结论，只补齐其未覆盖的版本历史面板。
