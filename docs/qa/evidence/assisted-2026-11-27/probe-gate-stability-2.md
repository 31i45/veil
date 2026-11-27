# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。

- 时间：2026-11-28 01:31:02 +08:00
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
- 探针总耗时：395.6 s
- 结果：**PASS**（无 FAIL）

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
| pathological_eval_child/loop | 病态 | PASS | 1.69 s | answer@loop: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_eval_child/microtask | 病态 | PASS | 1.71 s | answer@microtask: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_inputs_matrix | 病态 | PASS | 1.97 s | cases_passed: 10/10/10/10 count（min/p50/p95/max）；max_ms: 1005.1/1005.1/1005.1/1005.1 ms（min/p50/p95/max） | — |
| mindmap_viewbox_debug | 诊断 | PASS | 1.65 s | viewbox_height: 159.8/159.8/159.8/159.8 px（min/p50/p95/max）；viewbox_width: 543/543/543/543 px（min/p50/p95/max） | — |
| generate_wave4_sample_pdf | 证据 | PASS | 1.57 s | bytes: 252279/252279/252279/252279 byte（min/p50/p95/max）；outline: 14/14/14/14 count（min/p50/p95/max）；pages: 4/4/4/4 count（min/p50/p95/max） | — |
| perf_huge_scroll | 病态 | PASS | 5.17 s | doc_height: 20156/20156/20156/20156 px（min/p50/p95/max）；nonbg: 84898/84898/84898/84898 px（min/p50/p95/max） | — |
| perf_huge_block | 病态 | PASS | 36.82 s | app_ms@10mb: 120.58/127.286/131.435/131.793 ms（min/p50/p95/max）；app_ms@1mb: 15.907/16.627/17.802/18.321 ms（min/p50/p95/max）；app_ms@2mb: 25.583/26.644/31.591/32.338 ms（min/p50/p95/max）；edit_ms@10mb: 941.018/962.346/985.052/985.299 ms（min/p50/p95/max）；edit_ms@1mb: 148.248/154.691/159.084/160.033 ms（min/p50/p95/max）；edit_ms@2mb: 235.021/244.077/255.861/257.17 ms（min/p50/p95/max）；tail_ms@10mb: 950.535/968.756/991.584/993.2 ms（min/p50/p95/max）；tail_ms@1mb: 148.873/154.358/158.928/158.972 ms（min/p50/p95/max）；tail_ms@2mb: 237.55/242.934/248.609/249.578 ms（min/p50/p95/max） | — |
| bench_edit_reparse_history_100k | 性能 | PASS | 6.39 s | edit_us: 4.528/5.976/7.882/8.208 us（min/p50/p95/max）；history_us: 0.282/0.49/0.659/0.666 us（min/p50/p95/max）；reparse_us: 788.172/827.308/840.402/842.066 us（min/p50/p95/max） | reparse_us p50=827.308 us vs 基线 p50=834.39 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算）；edit_us p50=5.976 us vs 基线 p50=6.484 us：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2000 us，噪声地板生效（非产品预算） |
| perf_50mb_long_doc | 性能 | PASS | 21.45 s | blocks: 20001/20001/20001/20001 count（min/p50/p95/max）；keystroke_p50: 1.986/2.05/2.096/2.098 ms（min/p50/p95/max）；keystroke_p95: 2.181/2.376/2.814/3.018 ms（min/p50/p95/max）；open_ms: 479.5/494.7/504.52/508 ms（min/p50/p95/max）；save_ms: 340.7/348.6/353.58/355.5 ms（min/p50/p95/max） | keystroke_p95 p50=2.376 ms vs 基线 p50=2.364 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.837 ms，在阈值内；open_ms p95=504.52 ms vs 基线 p95=501.68 ms：常规类 p95 vs 基线 p95×1.2 = 602.016 ms，在阈值内 |
| perf_20k_blocks | 性能 | PASS | 19.47 s | blocks: 21000/21000/21000/21000 count（min/p50/p95/max）；keystroke_p50: 2.409/2.534/2.61/2.631 ms（min/p50/p95/max）；keystroke_p95: 2.828/2.976/3.104/3.131 ms（min/p50/p95/max） | keystroke_p95 p50=2.976 ms vs 基线 p50=2.741 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3.289 ms，在阈值内 |
| perf_keystroke_path | 性能 | PASS | 14.79 s | keystroke_p50: 1.249/1.331/1.423/1.424 ms（min/p50/p95/max）；keystroke_p95: 1.581/1.768/1.864/1.88 ms（min/p50/p95/max） | keystroke_p95 p50=1.768 ms vs 基线 p50=1.737 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.084 ms，在阈值内 |
| perf_keystroke_attribution | 性能 | PASS | 91.45 s | index_share@10003: 43.5/44.3/45.32/45.6 pct（min/p50/p95/max）；index_share@19999: 51.1/51.2/52.36/52.4 pct（min/p50/p95/max）；index_share@2002: 19.5/19.9/21/21 pct（min/p50/p95/max）；index_share@40005: 56.5/57/57.3/57.3 pct（min/p50/p95/max）；index_share@5005: 31.3/32/33.14/33.3 pct（min/p50/p95/max）；total_p95@10003: 4239.1/4671.5/4805/4863.4 us（min/p50/p95/max）；total_p95@19999: 7497.3/7857.4/8094.5/8108.8 us（min/p50/p95/max）；total_p95@2002: 1909.1/2050/2148.1/2154.3 us（min/p50/p95/max）；total_p95@40005: 14236.7/14551.7/14663.3/14691.7 us（min/p50/p95/max）；total_p95@5005: 2713.4/3035/3207.3/3221.4 us（min/p50/p95/max） | total_p95@10003 p50=4671.5 us vs 基线 p50=4306.4 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5167.7 us，在阈值内；total_p95@19999 p50=7857.4 us vs 基线 p50=7302.5 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 8763 us，在阈值内；total_p95@2002 p50=2050 us vs 基线 p50=2028.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2433.8 us，在阈值内；total_p95@40005 p50=14551.7 us vs 基线 p50=13497.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 16197.2 us，在阈值内；total_p95@5005 p50=3035 us vs 基线 p50=2976.1 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 3571.3 us，在阈值内 |
| perf_index_subsegments | 性能 | PASS | 76.2 s | index_sum_p95@10003: 2231/2449.4/2590.5/2636.5 us（min/p50/p95/max）；index_sum_p95@19999: 4598/4767.1/5032.8/5047.3 us（min/p50/p95/max）；index_sum_p95@2002: 249.6/291.2/330.2/333.8 us（min/p50/p95/max）；index_sum_p95@40005: 9392/9570/10084.4/10197.3 us（min/p50/p95/max）；index_sum_p95@5005: 986.1/1139.5/1413.9/1496.1 us（min/p50/p95/max） | index_sum_p95@10003 p50=2449.4 us vs 基线 p50=2249.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2699 us，在阈值内；index_sum_p95@19999 p50=4767.1 us vs 基线 p50=4376.9 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 5252.3 us，在阈值内；index_sum_p95@2002 p50=291.2 us vs 基线 p50=262.8 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1000 us，噪声地板生效（非产品预算）；index_sum_p95@40005 p50=9570 us vs 基线 p50=8789.2 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 10547 us，在阈值内；index_sum_p95@5005 p50=1139.5 us vs 基线 p50=1099.7 us：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1319.6 us，在阈值内 |
| perf_cold_first_edit | 性能 | PASS | 11.34 s | app_ms: 46.215/47.647/49.159/49.718 ms（min/p50/p95/max）；cold_first_edit_ms: 95.514/99.372/101.746/102.01 ms（min/p50/p95/max）；edit_ms: 48.156/51.691/53.626/53.702 ms（min/p50/p95/max） | cold_first_edit_ms p95=101.746 ms vs 基线 p95=105.775 ms：常规类 p95 vs 基线 p95×1.2 = 126.93 ms，在阈值内 |
| perf_cold_first_edit_prewarmed | 性能 | PASS | 11.21 s | edit_ms: 45.486/48.598/50.495/50.887 ms（min/p50/p95/max） | edit_ms p95=50.495 ms vs 基线 p95=53.24 ms：常规类 p95 vs 基线 p95×1.2 = 63.888 ms，在阈值内 |
| perf_first_edit_after_first_frame | 性能 | PASS | 11.25 s | first_edit_ms: 1.38/1.474/1.621/1.628 ms（min/p50/p95/max）；first_frame_ms: 51.201/53.781/57.85/58.192 ms（min/p50/p95/max） | first_edit_ms p50=1.474 ms vs 基线 p50=1.522 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2.283 ms，在阈值内 |
| perf_peak_breakdown | 性能 | PASS | 20.74 s | peak_paint_ms: 1.022/1.309/4.306/5.821 ms（min/p50/p95/max）；peak_total_ms: 2.89/3.445/7.873/8.991 ms（min/p50/p95/max） | peak_total_ms p50=3.445 ms vs 基线 p50=3.694 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.541 ms，在阈值内 |
| perf_after_edit_breakdown | 性能 | PASS | 27.99 s | p95_recompute_geometry_ms: 0.373/0.418/0.428/0.428 ms（min/p50/p95/max）；p95_refresh_index_ms: 2.052/2.15/2.27/2.298 ms（min/p50/p95/max）；peak_total_ms: 3.558/4.035/9.679/11.291 ms（min/p50/p95/max） | peak_total_ms p50=4.035 ms vs 基线 p50=3.742 ms：峰值类 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 5.613 ms，在阈值内；p95_refresh_index_ms p50=2.15 ms vs 基线 p50=1.988 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 2.386 ms，在阈值内 |
| perf_5000_headings | 性能 | PASS | 13.37 s | headings: 5000/5000/5000/5000 count（min/p50/p95/max）；headings_us: 19.8/31.6/35.88/36.4 us（min/p50/p95/max）；open_ms: 89.411/92.638/94.908/94.995 ms（min/p50/p95/max）；scroll_p50_ms: 0.739/0.78/0.808/0.812 ms（min/p50/p95/max）；scroll_p95_ms: 0.842/0.954/1.028/1.036 ms（min/p50/p95/max） | open_ms p95=94.908 ms vs 基线 p95=96.463 ms：常规类 p95 vs 基线 p95×1.2 = 115.755 ms，在阈值内；scroll_p95_ms p50=0.954 ms vs 基线 p50=0.979 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 1.175 ms，在阈值内 |
| perf_history_5000 | 性能 | PASS | 19.34 s | close_ms: 0.382/0.392/0.514/0.519 ms（min/p50/p95/max）；entries: 5000/5000/5000/5000 count（min/p50/p95/max）；open_ms: 9.017/9.446/10.514/10.721 ms（min/p50/p95/max）；scroll_p50_ms: 3.005/3.17/3.214/3.222 ms（min/p50/p95/max）；scroll_p95_ms: 3.265/3.534/3.646/3.653 ms（min/p50/p95/max） | open_ms p95=10.514 ms vs 基线 p95=10.738 ms：常规类 p95 vs 基线 p95×1.2 = 12.886 ms，在阈值内；scroll_p95_ms p50=3.534 ms vs 基线 p50=3.495 ms：探针内 p95 类 p50 vs 基线 p50，阈值 max(1 ms, 基线p50×1.2) = 4.194 ms，在阈值内；close_ms p50=0.392 ms vs 基线 p50=0.385 ms：小量级 p50 vs 基线 p50，阈值 max(2 ms, 基线p50×1.5) = 2 ms，噪声地板生效（非产品预算） |

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