# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。

- 时间：2026-11-28 01:24:17 +08:00
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
- 探针总耗时：397.3 s
- 结果：**PASS**（无 FAIL）

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
| pathological_eval_child/loop | 病态 | PASS | 1.74 s | answer@loop: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_eval_child/microtask | 病态 | PASS | 1.73 s | answer@microtask: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_inputs_matrix | 病态 | PASS | 1.97 s | cases_passed: 10/10/10/10 count（min/p50/p95/max）；max_ms: 1009.7/1009.7/1009.7/1009.7 ms（min/p50/p95/max） | — |
| mindmap_viewbox_debug | 诊断 | PASS | 1.71 s | viewbox_height: 159.8/159.8/159.8/159.8 px（min/p50/p95/max）；viewbox_width: 543/543/543/543 px（min/p50/p95/max） | — |
| generate_wave4_sample_pdf | 证据 | PASS | 1.52 s | bytes: 252279/252279/252279/252279 byte（min/p50/p95/max）；outline: 14/14/14/14 count（min/p50/p95/max）；pages: 4/4/4/4 count（min/p50/p95/max） | — |
| perf_huge_scroll | 病态 | PASS | 5.22 s | doc_height: 20156/20156/20156/20156 px（min/p50/p95/max）；nonbg: 84898/84898/84898/84898 px（min/p50/p95/max） | — |
| perf_huge_block | 病态 | PASS | 37.24 s | app_ms@10mb: 123.331/127.747/131.197/131.408 ms（min/p50/p95/max）；app_ms@1mb: 15.536/16.814/17.304/17.363 ms（min/p50/p95/max）；app_ms@2mb: 25.863/27.913/28.895/28.97 ms（min/p50/p95/max）；edit_ms@10mb: 941.583/981.907/993.133/995.687 ms（min/p50/p95/max）；edit_ms@1mb: 152.185/154.113/158.18/158.312 ms（min/p50/p95/max）；edit_ms@2mb: 237.643/248.03/255.322/259.241 ms（min/p50/p95/max）；tail_ms@10mb: 955.104/991.761/1011.5/1013.2 ms（min/p50/p95/max）；tail_ms@1mb: 150.849/155.127/159.918/160.755 ms（min/p50/p95/max）；tail_ms@2mb: 237.495/248.321/253.575/255.093 ms（min/p50/p95/max） | — |
| bench_edit_reparse_history_100k | 性能 | PASS | 6.43 s | edit_us: 5.682/6.202/7.942/8.132 us（min/p50/p95/max）；history_us: 0.468/0.598/0.86/0.902 us（min/p50/p95/max）；reparse_us: 820.304/836.012/1017.9/1125.6 us（min/p50/p95/max） | reparse_us p50=836.012 us vs 基线 p50=834.39 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算）；edit_us p50=6.202 us vs 基线 p50=6.484 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算） |
| perf_50mb_long_doc | 性能 | PASS | 21.66 s | blocks: 20001/20001/20001/20001 count（min/p50/p95/max）；keystroke_p50: 1.968/2.052/2.109/2.11 ms（min/p50/p95/max）；keystroke_p95: 2.192/2.367/2.604/2.619 ms（min/p50/p95/max）；open_ms: 479/495/499.22/499.3 ms（min/p50/p95/max）；save_ms: 344.2/352.3/366.08/371.4 ms（min/p50/p95/max） | keystroke_p95 p50=2.367 ms vs 基线 p50=2.364 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.837 ms，在阈值内；open_ms p95=499.22 ms vs 基线 p95=501.68 ms：常规类 p95 vs 基线 p95×1.2 = 602.016 ms，在阈值内 |
| perf_20k_blocks | 性能 | PASS | 19.37 s | blocks: 21000/21000/21000/21000 count（min/p50/p95/max）；keystroke_p50: 2.379/2.53/2.598/2.608 ms（min/p50/p95/max）；keystroke_p95: 2.738/3.002/3.18/3.224 ms（min/p50/p95/max） | keystroke_p95 p50=3.002 ms vs 基线 p50=2.741 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3.289 ms，在阈值内 |
| perf_keystroke_path | 性能 | PASS | 14.87 s | keystroke_p50: 1.298/1.334/1.4/1.416 ms（min/p50/p95/max）；keystroke_p95: 1.724/1.832/1.917/1.922 ms（min/p50/p95/max） | keystroke_p95 p50=1.832 ms vs 基线 p50=1.737 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.084 ms，在阈值内 |
| perf_keystroke_attribution | 性能 | PASS | 91.86 s | index_share@10003: 43.2/44.1/45.1/45.1 pct（min/p50/p95/max）；index_share@19999: 51/51.8/52.32/52.4 pct（min/p50/p95/max）；index_share@2002: 19/19.8/20.64/20.8 pct（min/p50/p95/max）；index_share@40005: 56/56.7/57.42/57.5 pct（min/p50/p95/max）；index_share@5005: 31.3/32.7/34.68/35 pct（min/p50/p95/max）；total_p95@10003: 4275.7/4674.1/4905/4922.6 us（min/p50/p95/max）；total_p95@19999: 7385.2/7817.2/8354/8401.8 us（min/p50/p95/max）；total_p95@2002: 1962.2/2114.6/2140.6/2142.6 us（min/p50/p95/max）；total_p95@40005: 14185.6/14588.6/14706.4/14749.8 us（min/p50/p95/max）；total_p95@5005: 2750.1/3107.6/3311.8/3384.2 us（min/p50/p95/max） | total_p95@10003 p50=4674.1 us vs 基线 p50=4306.4 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5167.7 us，在阈值内；total_p95@19999 p50=7817.2 us vs 基线 p50=7302.5 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 8763 us，在阈值内；total_p95@2002 p50=2114.6 us vs 基线 p50=2028.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2433.8 us，在阈值内；total_p95@40005 p50=14588.6 us vs 基线 p50=13497.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 16197.2 us，在阈值内；total_p95@5005 p50=3107.6 us vs 基线 p50=2976.1 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3571.3 us，在阈值内 |
| perf_index_subsegments | 性能 | PASS | 76.51 s | index_sum_p95@10003: 2425.8/2526.3/2646.9/2661.4 us（min/p50/p95/max）；index_sum_p95@19999: 4454.8/4898.3/5052.7/5060.8 us（min/p50/p95/max）；index_sum_p95@2002: 259.4/303.7/326.16/327 us（min/p50/p95/max）；index_sum_p95@40005: 9252.5/9564.3/9842.4/9870.6 us（min/p50/p95/max）；index_sum_p95@5005: 1098.5/1234.9/1448.5/1514.7 us（min/p50/p95/max） | index_sum_p95@10003 p50=2526.3 us vs 基线 p50=2249.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2699 us，在阈值内；index_sum_p95@19999 p50=4898.3 us vs 基线 p50=4376.9 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5252.3 us，在阈值内；index_sum_p95@2002 p50=303.7 us vs 基线 p50=262.8 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1000 us，噪声地板生效（非产品预算）；index_sum_p95@40005 p50=9564.3 us vs 基线 p50=8789.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 10547 us，在阈值内；index_sum_p95@5005 p50=1234.9 us vs 基线 p50=1099.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1319.6 us，在阈值内 |
| perf_cold_first_edit | 性能 | PASS | 11.43 s | app_ms: 46.354/47.421/48.866/48.874 ms（min/p50/p95/max）；cold_first_edit_ms: 95.704/97.911/105.933/109.173 ms（min/p50/p95/max）；edit_ms: 48.282/50.588/57.652/60.32 ms（min/p50/p95/max） | cold_first_edit_ms p95=105.933 ms vs 基线 p95=105.775 ms：常规类 p95 vs 基线 p95×1.2 = 126.93 ms，在阈值内 |
| perf_cold_first_edit_prewarmed | 性能 | PASS | 11.25 s | edit_ms: 46.113/49.306/52.088/52.854 ms（min/p50/p95/max） | edit_ms p95=52.088 ms vs 基线 p95=53.24 ms：常规类 p95 vs 基线 p95×1.2 = 63.888 ms，在阈值内 |
| perf_first_edit_after_first_frame | 性能 | PASS | 11.39 s | first_edit_ms: 1.367/1.432/1.822/1.988 ms（min/p50/p95/max）；first_frame_ms: 50.634/56.973/57.993/58.083 ms（min/p50/p95/max） | first_edit_ms p50=1.432 ms vs 基线 p50=1.522 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2.283 ms，在阈值内 |
| perf_peak_breakdown | 性能 | PASS | 20.69 s | peak_paint_ms: 0.739/1.557/3.824/4.793 ms（min/p50/p95/max）；peak_total_ms: 2.744/3.474/6.27/6.574 ms（min/p50/p95/max） | peak_total_ms p50=3.474 ms vs 基线 p50=3.694 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.541 ms，在阈值内 |
| perf_after_edit_breakdown | 性能 | PASS | 27.98 s | p95_recompute_geometry_ms: 0.372/0.415/0.444/0.456 ms（min/p50/p95/max）；p95_refresh_index_ms: 2/2.206/2.277/2.281 ms（min/p50/p95/max）；peak_total_ms: 3.425/3.714/5.526/5.767 ms（min/p50/p95/max） | peak_total_ms p50=3.714 ms vs 基线 p50=3.742 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.613 ms，在阈值内；p95_refresh_index_ms p50=2.206 ms vs 基线 p50=1.988 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.386 ms，在阈值内 |
| perf_5000_headings | 性能 | PASS | 13.45 s | headings: 5000/5000/5000/5000 count（min/p50/p95/max）；headings_us: 25.3/31.8/35.1/35.9 us（min/p50/p95/max）；open_ms: 89.622/93.57/99.268/101.446 ms（min/p50/p95/max）；scroll_p50_ms: 0.749/0.785/0.802/0.803 ms（min/p50/p95/max）；scroll_p95_ms: 0.885/0.969/1.054/1.054 ms（min/p50/p95/max） | open_ms p95=99.268 ms vs 基线 p95=96.463 ms：常规类 p95 vs 基线 p95×1.2 = 115.755 ms，在阈值内；scroll_p95_ms p50=0.969 ms vs 基线 p50=0.979 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1.175 ms，在阈值内 |
| perf_history_5000 | 性能 | PASS | 19.27 s | close_ms: 0.383/0.409/0.532/0.535 ms（min/p50/p95/max）；entries: 5000/5000/5000/5000 count（min/p50/p95/max）；open_ms: 9.185/9.711/9.905/9.939 ms（min/p50/p95/max）；scroll_p50_ms: 3.044/3.144/3.214/3.229 ms（min/p50/p95/max）；scroll_p95_ms: 3.265/3.599/3.823/3.869 ms（min/p50/p95/max） | open_ms p95=9.905 ms vs 基线 p95=10.738 ms：常规类 p95 vs 基线 p95×1.2 = 12.886 ms，在阈值内；scroll_p95_ms p50=3.599 ms vs 基线 p50=3.495 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 4.194 ms，在阈值内；close_ms p50=0.409 ms vs 基线 p50=0.385 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2 ms，噪声地板生效（非产品预算） |

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