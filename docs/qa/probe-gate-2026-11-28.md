# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。

- 时间：2026-11-28 01:37:44 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 构建：`cargo test --workspace --release --no-run`（耗时 2s）
- 采样：性能项与 `perf_huge_block`（大块耗时分布）`-Repeat 9`；其余非性能项固定 1 次
- 基线：`docs/qa/perf-baseline.json`（运行前已存在）
- 运行命令：`pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 9`

## 汇总

- 判定行：20（覆盖原始 17 条 ignored 探针 + Task 5 新增 2 条：19 条源码测试中 18 条仍为 `#[ignore]`，`mindmap_viewbox_debug` 已晋级为常规断言；`pathological_eval_child` 在门禁中以 loop / microtask 两种模式各判定一次）
- PASS：20 / FAIL：0 / SKIP：0
- 基线写入：0 项（首次运行或 `-UpdateBaseline` 时写入）
- 探针总耗时：397.5 s
- 结果：**PASS**（无 FAIL）

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
| pathological_eval_child/loop | 病态 | PASS | 1.78 s | answer@loop: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_eval_child/microtask | 病态 | PASS | 1.72 s | answer@microtask: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_inputs_matrix | 病态 | PASS | 1.97 s | cases_passed: 10/10/10/10 count（min/p50/p95/max）；max_ms: 1000.2/1000.2/1000.2/1000.2 ms（min/p50/p95/max） | — |
| mindmap_viewbox_debug | 诊断 | PASS | 1.7 s | viewbox_height: 159.8/159.8/159.8/159.8 px（min/p50/p95/max）；viewbox_width: 543/543/543/543 px（min/p50/p95/max） | — |
| generate_wave4_sample_pdf | 证据 | PASS | 1.58 s | bytes: 252279/252279/252279/252279 byte（min/p50/p95/max）；outline: 14/14/14/14 count（min/p50/p95/max）；pages: 4/4/4/4 count（min/p50/p95/max） | — |
| perf_huge_scroll | 病态 | PASS | 5.26 s | doc_height: 20156/20156/20156/20156 px（min/p50/p95/max）；nonbg: 84898/84898/84898/84898 px（min/p50/p95/max） | — |
| perf_huge_block | 病态 | PASS | 37.03 s | app_ms@10mb: 123.006/127.448/131.955/132.864 ms（min/p50/p95/max）；app_ms@1mb: 15.788/16.783/17.344/17.36 ms（min/p50/p95/max）；app_ms@2mb: 24.64/26.281/27.964/28.06 ms（min/p50/p95/max）；edit_ms@10mb: 932.787/976.936/993.037/994.354 ms（min/p50/p95/max）；edit_ms@1mb: 151.784/157.851/161.355/162.22 ms（min/p50/p95/max）；edit_ms@2mb: 238.636/245.948/253.178/255.02 ms（min/p50/p95/max）；tail_ms@10mb: 952.24/976.281/997.436/998.297 ms（min/p50/p95/max）；tail_ms@1mb: 149.637/155.053/161.891/164.101 ms（min/p50/p95/max）；tail_ms@2mb: 236.785/242.734/247.343/248.087 ms（min/p50/p95/max） | — |
| bench_edit_reparse_history_100k | 性能 | PASS | 6.4 s | edit_us: 5.028/6.484/7.598/7.852 us（min/p50/p95/max）；history_us: 0.34/0.598/0.834/0.912 us（min/p50/p95/max）；reparse_us: 814.728/829.518/868.07/874.768 us（min/p50/p95/max） | reparse_us p50=829.518 us vs 基线 p50=834.39 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算）；edit_us p50=6.484 us vs 基线 p50=6.484 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算） |
| perf_50mb_long_doc | 性能 | PASS | 21.65 s | blocks: 20001/20001/20001/20001 count（min/p50/p95/max）；keystroke_p50: 1.965/2.057/2.089/2.089 ms（min/p50/p95/max）；keystroke_p95: 2.208/2.41/2.547/2.569 ms（min/p50/p95/max）；open_ms: 483.6/494.9/501.06/501.7 ms（min/p50/p95/max）；save_ms: 342/349/355.66/357.3 ms（min/p50/p95/max） | keystroke_p95 p50=2.41 ms vs 基线 p50=2.364 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.837 ms，在阈值内；open_ms p95=501.06 ms vs 基线 p95=501.68 ms：常规类 p95 vs 基线 p95×1.2 = 602.016 ms，在阈值内 |
| perf_20k_blocks | 性能 | PASS | 19.4 s | blocks: 21000/21000/21000/21000 count（min/p50/p95/max）；keystroke_p50: 2.4/2.546/2.591/2.611 ms（min/p50/p95/max）；keystroke_p95: 2.754/2.946/3.05/3.068 ms（min/p50/p95/max） | keystroke_p95 p50=2.946 ms vs 基线 p50=2.741 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3.289 ms，在阈值内 |
| perf_keystroke_path | 性能 | PASS | 15.1 s | keystroke_p50: 1.286/1.374/1.406/1.414 ms（min/p50/p95/max）；keystroke_p95: 1.622/1.824/1.884/1.914 ms（min/p50/p95/max） | keystroke_p95 p50=1.824 ms vs 基线 p50=1.737 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.084 ms，在阈值内 |
| perf_keystroke_attribution | 性能 | PASS | 92.23 s | index_share@10003: 42.6/44.3/45.16/45.2 pct（min/p50/p95/max）；index_share@19999: 50.7/51.6/52.16/52.4 pct（min/p50/p95/max）；index_share@2002: 19.5/20.4/20.98/21.1 pct（min/p50/p95/max）；index_share@40005: 56.5/57/57.52/57.6 pct（min/p50/p95/max）；index_share@5005: 30.7/32.6/33.12/33.2 pct（min/p50/p95/max）；total_p95@10003: 4325.6/4724.7/5104/5199.2 us（min/p50/p95/max）；total_p95@19999: 7540.6/7920.1/8140.7/8199.5 us（min/p50/p95/max）；total_p95@2002: 1918/2124.7/2206.8/2210.2 us（min/p50/p95/max）；total_p95@40005: 14351.4/14622.5/14865.1/14917 us（min/p50/p95/max）；total_p95@5005: 2690.4/3251.2/3549/3685.4 us（min/p50/p95/max） | total_p95@10003 p50=4724.7 us vs 基线 p50=4306.4 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5167.7 us，在阈值内；total_p95@19999 p50=7920.1 us vs 基线 p50=7302.5 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 8763 us，在阈值内；total_p95@2002 p50=2124.7 us vs 基线 p50=2028.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2433.8 us，在阈值内；total_p95@40005 p50=14622.5 us vs 基线 p50=13497.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 16197.2 us，在阈值内；total_p95@5005 p50=3251.2 us vs 基线 p50=2976.1 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3571.3 us，在阈值内 |
| perf_index_subsegments | 性能 | PASS | 76.19 s | index_sum_p95@10003: 2347.1/2537.2/2700.2/2706.4 us（min/p50/p95/max）；index_sum_p95@19999: 4390.9/4721.3/4935.8/4982 us（min/p50/p95/max）；index_sum_p95@2002: 253.5/301.3/310.34/310.9 us（min/p50/p95/max）；index_sum_p95@40005: 9307.6/9666.6/10135.8/10196.9 us（min/p50/p95/max）；index_sum_p95@5005: 1093.1/1156.3/1224.9/1233.7 us（min/p50/p95/max） | index_sum_p95@10003 p50=2537.2 us vs 基线 p50=2249.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2699 us，在阈值内；index_sum_p95@19999 p50=4721.3 us vs 基线 p50=4376.9 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5252.3 us，在阈值内；index_sum_p95@2002 p50=301.3 us vs 基线 p50=262.8 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1000 us，噪声地板生效（非产品预算）；index_sum_p95@40005 p50=9666.6 us vs 基线 p50=8789.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 10547 us，在阈值内；index_sum_p95@5005 p50=1156.3 us vs 基线 p50=1099.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1319.6 us，在阈值内 |
| perf_cold_first_edit | 性能 | PASS | 11.31 s | app_ms: 46.17/47.583/49.082/49.348 ms（min/p50/p95/max）；cold_first_edit_ms: 94.475/99.587/102.361/102.641 ms（min/p50/p95/max）；edit_ms: 48.305/51.184/55.396/55.492 ms（min/p50/p95/max） | cold_first_edit_ms p95=102.361 ms vs 基线 p95=105.775 ms：常规类 p95 vs 基线 p95×1.2 = 126.93 ms，在阈值内 |
| perf_cold_first_edit_prewarmed | 性能 | PASS | 11.29 s | edit_ms: 46.32/50.038/52.046/52.274 ms（min/p50/p95/max） | edit_ms p95=52.046 ms vs 基线 p95=53.24 ms：常规类 p95 vs 基线 p95×1.2 = 63.888 ms，在阈值内 |
| perf_first_edit_after_first_frame | 性能 | PASS | 11.33 s | first_edit_ms: 1.384/1.468/1.611/1.668 ms（min/p50/p95/max）；first_frame_ms: 53.654/55.168/57.671/58.284 ms（min/p50/p95/max） | first_edit_ms p50=1.468 ms vs 基线 p50=1.522 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2.283 ms，在阈值内 |
| perf_peak_breakdown | 性能 | PASS | 20.72 s | peak_paint_ms: 0.887/1.349/3.901/4.02 ms（min/p50/p95/max）；peak_total_ms: 2.997/3.464/6.137/6.885 ms（min/p50/p95/max） | peak_total_ms p50=3.464 ms vs 基线 p50=3.694 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.541 ms，在阈值内 |
| perf_after_edit_breakdown | 性能 | PASS | 27.93 s | p95_recompute_geometry_ms: 0.378/0.417/0.425/0.425 ms（min/p50/p95/max）；p95_refresh_index_ms: 2.021/2.144/2.271/2.274 ms（min/p50/p95/max）；peak_total_ms: 3.463/4.133/8.53/9.672 ms（min/p50/p95/max） | peak_total_ms p50=4.133 ms vs 基线 p50=3.742 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.613 ms，在阈值内；p95_refresh_index_ms p50=2.144 ms vs 基线 p50=1.988 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.386 ms，在阈值内 |
| perf_5000_headings | 性能 | PASS | 13.53 s | headings: 5000/5000/5000/5000 count（min/p50/p95/max）；headings_us: 29.5/32.8/43.32/47.8 us（min/p50/p95/max）；open_ms: 91.148/94.689/96.199/96.435 ms（min/p50/p95/max）；scroll_p50_ms: 0.748/0.786/0.799/0.8 ms（min/p50/p95/max）；scroll_p95_ms: 0.881/0.972/1.017/1.025 ms（min/p50/p95/max） | open_ms p95=96.199 ms vs 基线 p95=96.463 ms：常规类 p95 vs 基线 p95×1.2 = 115.755 ms，在阈值内；scroll_p95_ms p50=0.972 ms vs 基线 p50=0.979 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1.175 ms，在阈值内 |
| perf_history_5000 | 性能 | PASS | 19.34 s | close_ms: 0.387/0.395/0.549/0.633 ms（min/p50/p95/max）；entries: 5000/5000/5000/5000 count（min/p50/p95/max）；open_ms: 9.123/9.745/10.101/10.268 ms（min/p50/p95/max）；scroll_p50_ms: 2.998/3.128/3.191/3.198 ms（min/p50/p95/max）；scroll_p95_ms: 3.365/3.559/3.666/3.687 ms（min/p50/p95/max） | open_ms p95=10.101 ms vs 基线 p95=10.738 ms：常规类 p95 vs 基线 p95×1.2 = 12.886 ms，在阈值内；scroll_p95_ms p50=3.559 ms vs 基线 p50=3.495 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 4.194 ms，在阈值内；close_ms p50=0.395 ms vs 基线 p50=0.385 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2 ms，噪声地板生效（非产品预算） |

## 判定口径

### 测量统计量（跨样本，p50/p95 均为线性插值 `rank=(n-1)*q`）
- 指标名含 `peak`/`max`（单次峰值类）：本轮 **p50** vs 基线 **p50**，阈值 `max(2 ms, 基线 p50 × 1.5)`。
- 指标名含 `p95`（探针内部 p95）：本轮 **p50** vs 基线 **p50**，阈值 `max(1 ms, 基线 p50 × 1.2)`。
- 其余指标：基线 p95 ≥ 5 ms → 本轮 **p95** vs 基线 p95 × 1.2；
  基线 p95 < 5 ms → 本轮 **p50** vs 基线 p50，阈值 `max(2 ms, 基线 p50 × 1.5)`。
- 基线 `docs/qa/perf-baseline.json` 为 schema 3，同时保存 `p50` 与 `p95`（`value` 兼容保留为 p95）。

### 测量噪声地板 ≠ 产品预算
- 上述 `max(…, 固定毫秒)` 是**测量噪声地板**，只吸收计时分辨率与调度抖动，仅用于回归比较。
- 探针自身写死的**产品预算**（`perf_50mb_long_doc` 打开 ≤ 5000 ms、击键 p95 ≤ 16 ms）硬性优先：
  超预算直接 FAIL，噪声地板不能豁免；本轮不修改任何产品预算或探针内断言。
- 非性能项：退出码 0 + 关键不变量（`PROBE` 指标等于/不低于下限、CJK 证据文件存在且非空）。
- SKIP 仅用于环境缺失（缺少系统 CJK 字体、50MB 探针内存分配失败），必须写明原因，绝不记 PASS。
- 每条命令超时 300–900 s（默认 600 s），超时记 FAIL 并保留输出摘要。

## 失败日志摘要
无。