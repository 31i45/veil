# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。

- 时间：2026-11-28 02:19:47 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 构建：`cargo test --workspace --release --no-run`（耗时 1.9s）
- 采样：性能项与 `perf_huge_block`（大块耗时分布）`-Repeat 9`；其余非性能项固定 1 次
- 基线：`docs/qa/perf-baseline.json`（运行前已存在）
- 运行命令：`pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 9`

## 汇总

- 判定行：20（覆盖原始 17 条 ignored 探针 + Task 5 新增 2 条：19 条源码测试中 18 条仍为 `#[ignore]`，`mindmap_viewbox_debug` 已晋级为常规断言；`pathological_eval_child` 在门禁中以 loop / microtask 两种模式各判定一次）
- PASS：20 / FAIL：0 / SKIP：0
- 基线写入：0 项（首次运行或 `-UpdateBaseline` 时写入）
- 探针总耗时：399.2 s
- 结果：**PASS**（无 FAIL）

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
| pathological_eval_child/loop | 病态 | PASS | 1.77 s | answer@loop: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_eval_child/microtask | 病态 | PASS | 1.74 s | answer@microtask: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_inputs_matrix | 病态 | PASS | 1.94 s | cases_passed: 10/10/10/10 count（min/p50/p95/max）；max_ms: 979.77/979.77/979.77/979.77 ms（min/p50/p95/max） | — |
| mindmap_viewbox_debug | 诊断 | PASS | 1.69 s | viewbox_height: 159.8/159.8/159.8/159.8 px（min/p50/p95/max）；viewbox_width: 543/543/543/543 px（min/p50/p95/max） | — |
| generate_wave4_sample_pdf | 证据 | PASS | 1.58 s | bytes: 252279/252279/252279/252279 byte（min/p50/p95/max）；outline: 14/14/14/14 count（min/p50/p95/max）；pages: 4/4/4/4 count（min/p50/p95/max） | — |
| perf_huge_scroll | 病态 | PASS | 5.28 s | doc_height: 20156/20156/20156/20156 px（min/p50/p95/max）；nonbg: 84898/84898/84898/84898 px（min/p50/p95/max） | — |
| perf_huge_block | 病态 | PASS | 37.51 s | app_ms@10mb: 123.538/129.436/130.954/131.117 ms（min/p50/p95/max）；app_ms@1mb: 14.797/17.098/19.134/19.918 ms（min/p50/p95/max）；app_ms@2mb: 25.409/27.809/30.06/30.224 ms（min/p50/p95/max）；edit_ms@10mb: 950.378/989.226/1032.9/1034.5 ms（min/p50/p95/max）；edit_ms@1mb: 152.742/157.363/161.936/162.738 ms（min/p50/p95/max）；edit_ms@2mb: 234.251/247.352/261.299/264.991 ms（min/p50/p95/max）；tail_ms@10mb: 961.562/989.383/1012.6/1015.9 ms（min/p50/p95/max）；tail_ms@1mb: 150.119/155.82/163.456/166.18 ms（min/p50/p95/max）；tail_ms@2mb: 236.374/244.889/253.34/256.382 ms（min/p50/p95/max） | — |
| bench_edit_reparse_history_100k | 性能 | PASS | 6.43 s | edit_us: 4.668/6.426/7.249/7.544 us（min/p50/p95/max）；history_us: 0.274/0.626/0.728/0.768 us（min/p50/p95/max）；reparse_us: 812.554/844.46/855.586/860.114 us（min/p50/p95/max） | reparse_us p50=844.46 us vs 基线 p50=834.39 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算）；edit_us p50=6.426 us vs 基线 p50=6.484 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算） |
| perf_50mb_long_doc | 性能 | PASS | 21.7 s | blocks: 20001/20001/20001/20001 count（min/p50/p95/max）；keystroke_p50: 1.969/2.044/2.108/2.12 ms（min/p50/p95/max）；keystroke_p95: 2.148/2.302/2.561/2.599 ms（min/p50/p95/max）；open_ms: 476.6/500.1/514.68/514.8 ms（min/p50/p95/max）；save_ms: 342.1/355.3/364.48/365.6 ms（min/p50/p95/max） | keystroke_p95 p50=2.302 ms vs 基线 p50=2.364 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.837 ms，在阈值内；open_ms p95=514.68 ms vs 基线 p95=501.68 ms：常规类 p95 vs 基线 p95×1.2 = 602.016 ms，在阈值内 |
| perf_20k_blocks | 性能 | PASS | 19.55 s | blocks: 21000/21000/21000/21000 count（min/p50/p95/max）；keystroke_p50: 2.329/2.533/2.63/2.646 ms（min/p50/p95/max）；keystroke_p95: 2.783/2.989/3.157/3.163 ms（min/p50/p95/max） | keystroke_p95 p50=2.989 ms vs 基线 p50=2.741 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3.289 ms，在阈值内 |
| perf_keystroke_path | 性能 | PASS | 15.1 s | keystroke_p50: 1.304/1.353/1.407/1.411 ms（min/p50/p95/max）；keystroke_p95: 1.681/1.791/1.937/1.952 ms（min/p50/p95/max） | keystroke_p95 p50=1.791 ms vs 基线 p50=1.737 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.084 ms，在阈值内 |
| perf_keystroke_attribution | 性能 | PASS | 92.17 s | index_share@10003: 43.5/44.5/45.26/45.5 pct（min/p50/p95/max）；index_share@19999: 50.4/51.3/52.44/52.6 pct（min/p50/p95/max）；index_share@2002: 18.6/19.8/20.78/20.9 pct（min/p50/p95/max）；index_share@40005: 56.2/56.8/57.16/57.2 pct（min/p50/p95/max）；index_share@5005: 30.9/32/33.58/34.1 pct（min/p50/p95/max）；total_p95@10003: 4309.9/4874/4985.8/5033 us（min/p50/p95/max）；total_p95@19999: 7518.5/7942/8253.2/8315.6 us（min/p50/p95/max）；total_p95@2002: 1927.7/2129.2/2262.8/2301.2 us（min/p50/p95/max）；total_p95@40005: 14224.1/14541.4/15067.5/15212 us（min/p50/p95/max）；total_p95@5005: 2686.7/3038.5/3509.2/3685.3 us（min/p50/p95/max） | total_p95@10003 p50=4874 us vs 基线 p50=4306.4 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5167.7 us，在阈值内；total_p95@19999 p50=7942 us vs 基线 p50=7302.5 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 8763 us，在阈值内；total_p95@2002 p50=2129.2 us vs 基线 p50=2028.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2433.8 us，在阈值内；total_p95@40005 p50=14541.4 us vs 基线 p50=13497.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 16197.2 us，在阈值内；total_p95@5005 p50=3038.5 us vs 基线 p50=2976.1 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3571.3 us，在阈值内 |
| perf_index_subsegments | 性能 | PASS | 76.59 s | index_sum_p95@10003: 2201.1/2518.9/2717.4/2745.6 us（min/p50/p95/max）；index_sum_p95@19999: 4404.6/4718.4/5096.8/5119.5 us（min/p50/p95/max）；index_sum_p95@2002: 251.7/301.4/327.7/328.7 us（min/p50/p95/max）；index_sum_p95@40005: 9455.2/9735/10021.5/10067.4 us（min/p50/p95/max）；index_sum_p95@5005: 1096.9/1231.5/1390.3/1403.7 us（min/p50/p95/max） | index_sum_p95@10003 p50=2518.9 us vs 基线 p50=2249.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2699 us，在阈值内；index_sum_p95@19999 p50=4718.4 us vs 基线 p50=4376.9 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5252.3 us，在阈值内；index_sum_p95@2002 p50=301.4 us vs 基线 p50=262.8 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1000 us，噪声地板生效（非产品预算）；index_sum_p95@40005 p50=9735 us vs 基线 p50=8789.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 10547 us，在阈值内；index_sum_p95@5005 p50=1231.5 us vs 基线 p50=1099.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1319.6 us，在阈值内 |
| perf_cold_first_edit | 性能 | PASS | 11.51 s | app_ms: 46.944/48.257/50.119/50.137 ms（min/p50/p95/max）；cold_first_edit_ms: 95.336/99.457/102.718/103.532 ms（min/p50/p95/max）；edit_ms: 48.392/51.143/54.442/55.243 ms（min/p50/p95/max） | cold_first_edit_ms p95=102.718 ms vs 基线 p95=105.775 ms：常规类 p95 vs 基线 p95×1.2 = 126.93 ms，在阈值内 |
| perf_cold_first_edit_prewarmed | 性能 | PASS | 11.42 s | edit_ms: 45.499/48.947/52.224/53.011 ms（min/p50/p95/max） | edit_ms p95=52.224 ms vs 基线 p95=53.24 ms：常规类 p95 vs 基线 p95×1.2 = 63.888 ms，在阈值内 |
| perf_first_edit_after_first_frame | 性能 | PASS | 11.59 s | first_edit_ms: 1.323/1.442/1.504/1.508 ms（min/p50/p95/max）；first_frame_ms: 51.176/54.836/55.835/55.837 ms（min/p50/p95/max） | first_edit_ms p50=1.442 ms vs 基线 p50=1.522 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2.283 ms，在阈值内 |
| perf_peak_breakdown | 性能 | PASS | 20.81 s | peak_paint_ms: 0.635/2.094/8.008/10.387 ms（min/p50/p95/max）；peak_total_ms: 2.949/4.01/13.424/18.758 ms（min/p50/p95/max） | peak_total_ms p50=4.01 ms vs 基线 p50=3.694 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.541 ms，在阈值内 |
| perf_after_edit_breakdown | 性能 | PASS | 27.99 s | p95_recompute_geometry_ms: 0.361/0.411/0.422/0.423 ms（min/p50/p95/max）；p95_refresh_index_ms: 1.993/2.134/2.215/2.223 ms（min/p50/p95/max）；peak_total_ms: 3.409/3.621/4.957/5.043 ms（min/p50/p95/max） | peak_total_ms p50=3.621 ms vs 基线 p50=3.742 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.613 ms，在阈值内；p95_refresh_index_ms p50=2.134 ms vs 基线 p50=1.988 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.386 ms，在阈值内 |
| perf_5000_headings | 性能 | PASS | 13.31 s | headings: 5000/5000/5000/5000 count（min/p50/p95/max）；headings_us: 28.8/32.6/38.96/41 us（min/p50/p95/max）；open_ms: 89.688/93.111/100.234/103.15 ms（min/p50/p95/max）；scroll_p50_ms: 0.739/0.771/0.809/0.814 ms（min/p50/p95/max）；scroll_p95_ms: 0.865/0.915/1.036/1.072 ms（min/p50/p95/max） | open_ms p95=100.234 ms vs 基线 p95=96.463 ms：常规类 p95 vs 基线 p95×1.2 = 115.755 ms，在阈值内；scroll_p95_ms p50=0.915 ms vs 基线 p50=0.979 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1.175 ms，在阈值内 |
| perf_history_5000 | 性能 | PASS | 19.51 s | close_ms: 0.387/0.415/1.06/1.457 ms（min/p50/p95/max）；entries: 5000/5000/5000/5000 count（min/p50/p95/max）；open_ms: 9.137/9.753/10.213/10.214 ms（min/p50/p95/max）；scroll_p50_ms: 3.069/3.165/3.254/3.258 ms（min/p50/p95/max）；scroll_p95_ms: 3.423/3.615/3.707/3.74 ms（min/p50/p95/max） | open_ms p95=10.213 ms vs 基线 p95=10.738 ms：常规类 p95 vs 基线 p95×1.2 = 12.886 ms，在阈值内；scroll_p95_ms p50=3.615 ms vs 基线 p50=3.495 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 4.194 ms，在阈值内；close_ms p50=0.415 ms vs 基线 p50=0.385 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2 ms，噪声地板生效（非产品预算） |

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