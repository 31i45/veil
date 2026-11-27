# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。
> 判定在探针内部完成（测试内断言回归上限/结构不变量）；本脚本只运行与归档。

- 时间：2026-11-28 07:30:21 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 结果：**PASS**（cargo exit 0）
- 构建耗时：2.2 s
- 运行耗时：38.4 s
- 串行执行：`RUST_TEST_THREADS=1`（避免性能探针互相抢 CPU）
- 失败用例总数（解析 `test result:` 行）：0

## 命令

```text
cargo test --workspace --release --no-run
cargo test --workspace --release -- --ignored --nocapture
```

## 探针结果行

- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 288 filtered out; finished in 0.10s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.00s`
- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 368 filtered out; finished in 0.06s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 86 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s`
- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.29s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s`
- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 29 filtered out; finished in 0.00s`
- `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1475 filtered out; finished in 32.42s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`

## 输出尾部（最后 120 行）

```text
PROBE name=perf_index_subsegments metric=index_sum_p95 value=4338.30 unit=us scope=19999
40005,1564.50,1759.70,4637.60,5025.00,1572.40,1736.50,0.00,0.10,7774.50,8521.30,20.7,59.0,20.4,0.0
PROBE name=perf_index_subsegments metric=index_sum_p95 value=8521.30 unit=us scope=40005
ok
test app_tests::render::perf_keystroke_attribution ... # segments: core_apply_reparse=Editor::apply->Session::apply; index_rebuild=App::refresh_index（含 recompute_geometry + refresh_stats）; dirty_layout=App::ensure_block（真实缓存查询+排版）; rasterize=LayoutEngine::rasterize（脏块）; frame_compose=App::render（真实 headless 帧合成，present 无 surface 不测）
doc_blocks,core_apply_reparse_p50_us,core_apply_reparse_p95_us,index_rebuild_p50_us,index_rebuild_p95_us,dirty_layout_p50_us,dirty_layout_p95_us,rasterize_p50_us,rasterize_p95_us,frame_compose_p50_us,frame_compose_p95_us,total_p95_us,index_share_pct
2002,85.40,111.10,185.10,274.10,84.00,138.30,145.90,271.20,664.00,857.30,1652.00,16.6
PROBE name=perf_keystroke_attribution metric=total_p95 value=1652.00 unit=us scope=2002
PROBE name=perf_keystroke_attribution metric=index_share value=16.6 unit=pct scope=2002
5005,209.10,355.30,488.50,801.90,95.80,162.40,152.90,314.30,784.00,1024.20,2658.10,30.2
PROBE name=perf_keystroke_attribution metric=total_p95 value=2658.10 unit=us scope=5005
PROBE name=perf_keystroke_attribution metric=index_share value=30.2 unit=pct scope=5005
10003,731.10,848.00,1687.20,1915.20,106.00,157.80,147.70,260.90,985.00,1132.40,4314.30,44.4
PROBE name=perf_keystroke_attribution metric=total_p95 value=4314.30 unit=us scope=10003
PROBE name=perf_keystroke_attribution metric=index_share value=44.4 unit=pct scope=10003
19999,1266.10,1412.40,3307.40,3535.50,113.30,174.00,152.00,275.30,1361.00,1538.40,6935.60,51.0
PROBE name=perf_keystroke_attribution metric=total_p95 value=6935.60 unit=us scope=19999
PROBE name=perf_keystroke_attribution metric=index_share value=51.0 unit=pct scope=19999
40005,2571.30,2866.70,6894.60,7527.90,133.20,180.00,166.00,282.40,2155.00,2347.10,13204.10,57.0
PROBE name=perf_keystroke_attribution metric=total_p95 value=13204.10 unit=us scope=40005
PROBE name=perf_keystroke_attribution metric=index_share value=57.0 unit=pct scope=40005
ok
test app_tests::render::perf_keystroke_path ... perf_blocks=5005
perf_keystroke_ms_p50=1.013
perf_keystroke_ms_p95=1.281
perf_keystroke_ms_max=1.695
PROBE name=perf_keystroke_path metric=keystroke_p50 value=1.013 unit=ms
PROBE name=perf_keystroke_path metric=keystroke_p95 value=1.281 unit=ms
ok
test app_tests::view::perf_after_edit_breakdown ... after_warmup_cache_layouts=18
iter0_cache_size=17
iter1_cache_size=18
iter2_cache_size=18
iter3_cache_size=18
iter4_cache_size=18
peak_total_ms=5.433
peak_refresh_index_ms=4.879
peak_ensure_layouts_visible_ms=0.224
peak_ensure_block_ms=0.001
peak_ensure_tables_visible_ms=0.001
peak_recompute_geometry_ms=0.327
peak_scroll_to_caret_ms=0.001
peak_dirty_count=1
peak_visible_range_len=12
max_dirty_count=1
peak_missing_layouts=1
max_missing_layouts=1
iters_with_multi_missing=0/500
iter0_missing=1
iter1_missing=1
iter2_missing=1
iter3_missing=1
iter4_missing=1
iter5_missing=1
iter6_missing=1
iter7_missing=1
iter8_missing=1
iter9_missing=1
iter10_missing=1
iter11_missing=1
iter12_missing=1
iter13_missing=1
iter14_missing=1
iter15_missing=1
iter16_missing=1
iter17_missing=1
iter18_missing=1
iter19_missing=1
before_ids=[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
after_ids=[10500, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]
ids_changed_in_visible_range=1
p95_refresh_index_ms=1.921
p95_ensure_layouts_visible_ms=0.232
p95_recompute_geometry_ms=0.391
PROBE name=perf_after_edit_breakdown metric=peak_total_ms value=5.433 unit=ms
PROBE name=perf_after_edit_breakdown metric=p95_refresh_index_ms value=1.921 unit=ms
PROBE name=perf_after_edit_breakdown metric=p95_recompute_geometry_ms value=0.391 unit=ms
ok
test app_tests::view::perf_peak_breakdown ... peak_total_ms=3.357
peak_apply_ms=0.780
peak_after_edit_ms=0.560
peak_paint_ms=2.017
paint_spikes_gt5ms=0/500
after_edit_spikes_gt5ms=0/500
PROBE name=perf_peak_breakdown metric=peak_total_ms value=3.357 unit=ms
PROBE name=perf_peak_breakdown metric=peak_paint_ms value=2.017 unit=ms
ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1475 filtered out; finished in 32.42s

[1m[92m     Running[0m tests\boundary_guard.rs (target\release\deps\boundary_guard-186c09e155ac9046.exe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s

[1m[92m   Doc-tests[0m veil

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

[1m[92m   Doc-tests[0m veil_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

[1m[92m   Doc-tests[0m veil_mermaid_offline

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

[1m[92m   Doc-tests[0m veil_ui

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```