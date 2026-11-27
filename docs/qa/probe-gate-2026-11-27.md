# Probe gate 结果（2026-11-27）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。

- 时间：2026-11-27 23:23:14 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 构建：`cargo test --workspace --release --no-run`（耗时 2.1s）
- 采样：性能项 `-Repeat 1`；非性能项固定 1 次
- 基线：`docs/qa/perf-baseline.json`（运行前已存在；本轮 -UpdateBaseline 重写）
- 运行命令：`pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 1 -UpdateBaseline`

## 汇总

- 判定行：18（覆盖 17 条原始 ignored 探针：16 条仍为 `#[ignore]`，`mindmap_viewbox_debug` 已晋级为常规断言；`pathological_eval_child` 在门禁中以 loop / microtask 两种模式各判定一次）
- PASS：18 / FAIL：0 / SKIP：0
- 基线写入：22 项（首次运行或 `-UpdateBaseline` 时写入）
- 探针总耗时：52.2 s
- 结果：**PASS**（无 FAIL）

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
| pathological_eval_child/loop | 病态 | PASS | 1.75 s | answer@loop: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_eval_child/microtask | 病态 | PASS | 1.75 s | answer@microtask: 42/42/42/42 int（min/p50/p95/max） | — |
| pathological_inputs_matrix | 病态 | PASS | 1.99 s | cases_passed: 10/10/10/10 count（min/p50/p95/max）；max_ms: 998.76/998.76/998.76/998.76 ms（min/p50/p95/max） | — |
| mindmap_viewbox_debug | 诊断 | PASS | 1.72 s | viewbox_height: 159.8/159.8/159.8/159.8 px（min/p50/p95/max）；viewbox_width: 543/543/543/543 px（min/p50/p95/max） | — |
| generate_wave4_sample_pdf | 证据 | PASS | 1.63 s | bytes: 252279/252279/252279/252279 byte（min/p50/p95/max）；outline: 14/14/14/14 count（min/p50/p95/max）；pages: 4/4/4/4 count（min/p50/p95/max） | — |
| perf_huge_scroll | 病态 | PASS | 4.98 s | doc_height: 20156/20156/20156/20156 px（min/p50/p95/max）；nonbg: 84898/84898/84898/84898 px（min/p50/p95/max） | — |
| perf_huge_block | 病态 | PASS | 4.09 s | app_ms@10mb: 118.78/118.78/118.78/118.78 ms（min/p50/p95/max）；app_ms@1mb: 17.858/17.858/17.858/17.858 ms（min/p50/p95/max）；app_ms@2mb: 26.098/26.098/26.098/26.098 ms（min/p50/p95/max）；edit_ms@10mb: 957.165/957.165/957.165/957.165 ms（min/p50/p95/max）；edit_ms@1mb: 157.994/157.994/157.994/157.994 ms（min/p50/p95/max）；edit_ms@2mb: 245.537/245.537/245.537/245.537 ms（min/p50/p95/max）；tail_ms@10mb: 923.036/923.036/923.036/923.036 ms（min/p50/p95/max）；tail_ms@1mb: 152.837/152.837/152.837/152.837 ms（min/p50/p95/max）；tail_ms@2mb: 242.915/242.915/242.915/242.915 ms（min/p50/p95/max） | — |
| bench_edit_reparse_history_100k | 性能 | PASS | 0.73 s | edit_us: 5.09/5.09/5.09/5.09 us（min/p50/p95/max）；history_us: 0.388/0.388/0.388/0.388 us（min/p50/p95/max）；reparse_us: 827.258/827.258/827.258/827.258 us（min/p50/p95/max） | — |
| perf_50mb_long_doc | 性能 | PASS | 2.46 s | blocks: 20001/20001/20001/20001 count（min/p50/p95/max）；keystroke_p50: 2.033/2.033/2.033/2.033 ms（min/p50/p95/max）；keystroke_p95: 2.284/2.284/2.284/2.284 ms（min/p50/p95/max）；open_ms: 488.8/488.8/488.8/488.8 ms（min/p50/p95/max）；save_ms: 332.4/332.4/332.4/332.4 ms（min/p50/p95/max） | — |
| perf_20k_blocks | 性能 | PASS | 2.18 s | blocks: 21000/21000/21000/21000 count（min/p50/p95/max）；keystroke_p50: 2.312/2.312/2.312/2.312 ms（min/p50/p95/max）；keystroke_p95: 2.586/2.586/2.586/2.586 ms（min/p50/p95/max） | — |
| perf_keystroke_path | 性能 | PASS | 1.62 s | keystroke_p50: 1.234/1.234/1.234/1.234 ms（min/p50/p95/max）；keystroke_p95: 1.827/1.827/1.827/1.827 ms（min/p50/p95/max） | — |
| perf_keystroke_attribution | 性能 | PASS | 9.79 s | index_share@10003: 44.2/44.2/44.2/44.2 pct（min/p50/p95/max）；index_share@19999: 51.3/51.3/51.3/51.3 pct（min/p50/p95/max）；index_share@2002: 20.5/20.5/20.5/20.5 pct（min/p50/p95/max）；index_share@40005: 56.5/56.5/56.5/56.5 pct（min/p50/p95/max）；index_share@5005: 31.7/31.7/31.7/31.7 pct（min/p50/p95/max）；total_p95@10003: 4708.2/4708.2/4708.2/4708.2 us（min/p50/p95/max）；total_p95@19999: 6991.9/6991.9/6991.9/6991.9 us（min/p50/p95/max）；total_p95@2002: 2004.4/2004.4/2004.4/2004.4 us（min/p50/p95/max）；total_p95@40005: 13695.3/13695.3/13695.3/13695.3 us（min/p50/p95/max）；total_p95@5005: 2926.2/2926.2/2926.2/2926.2 us（min/p50/p95/max） | — |
| perf_index_subsegments | 性能 | PASS | 8.06 s | index_sum_p95@10003: 2334/2334/2334/2334 us（min/p50/p95/max）；index_sum_p95@19999: 4670.5/4670.5/4670.5/4670.5 us（min/p50/p95/max）；index_sum_p95@2002: 304.3/304.3/304.3/304.3 us（min/p50/p95/max）；index_sum_p95@40005: 8900.9/8900.9/8900.9/8900.9 us（min/p50/p95/max）；index_sum_p95@5005: 1268.2/1268.2/1268.2/1268.2 us（min/p50/p95/max） | — |
| perf_cold_first_edit | 性能 | PASS | 1.37 s | app_ms: 48.882/48.882/48.882/48.882 ms（min/p50/p95/max）；cold_first_edit_ms: 101.627/101.627/101.627/101.627 ms（min/p50/p95/max）；edit_ms: 52.745/52.745/52.745/52.745 ms（min/p50/p95/max） | — |
| perf_cold_first_edit_prewarmed | 性能 | PASS | 1.36 s | edit_ms: 55.704/55.704/55.704/55.704 ms（min/p50/p95/max） | — |
| perf_first_edit_after_first_frame | 性能 | PASS | 1.29 s | first_edit_ms: 1.35/1.35/1.35/1.35 ms（min/p50/p95/max）；first_frame_ms: 51.077/51.077/51.077/51.077 ms（min/p50/p95/max） | — |
| perf_peak_breakdown | 性能 | PASS | 2.37 s | peak_paint_ms: 0.902/0.902/0.902/0.902 ms（min/p50/p95/max）；peak_total_ms: 3.335/3.335/3.335/3.335 ms（min/p50/p95/max） | — |
| perf_after_edit_breakdown | 性能 | PASS | 3.1 s | p95_recompute_geometry_ms: 0.405/0.405/0.405/0.405 ms（min/p50/p95/max）；p95_refresh_index_ms: 2.13/2.13/2.13/2.13 ms（min/p50/p95/max）；peak_total_ms: 3.371/3.371/3.371/3.371 ms（min/p50/p95/max） | — |

## 判定口径

- 性能项：本轮 p95 > 基线 p95 × 1.2 或 > 探针登记预算 → FAIL。预算只登记探针自身已写死的断言
  （`perf_50mb_long_doc` 打开 ≤ 5000 ms、击键 p95 ≤ 16 ms），其余以基线回归为准。
- 非性能项：退出码 0 + 关键不变量（`PROBE` 指标等于/不低于下限、CJK 证据文件存在且非空）。
- SKIP 仅用于环境缺失（缺少系统 CJK 字体、50MB 探针内存分配失败），必须写明原因，绝不记 PASS。
- 每条命令超时 300–900 s（默认 600 s），超时记 FAIL 并保留输出摘要。

## 失败日志摘要
无。