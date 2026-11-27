# Probe gate 结果（2026-11-28）

> 本文件由 `tools/verify/probe-gate.ps1` 自动生成，重复运行整体覆盖。
> 判定在探针内部完成（测试内断言回归上限/结构不变量）；本脚本只运行与归档。

- 时间：2026-11-28 07:20:05 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 结果：**PASS**（cargo exit 0）
- 构建耗时：2 s
- 运行耗时：38.2 s
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
- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.28s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s`
- `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 29 filtered out; finished in 0.00s`
- `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1475 filtered out; finished in 32.30s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`
- `test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`

## 输出尾部（最后 120 行）

```text
PROBE name=perf_index_subsegments metric=index_sum_p95 value=4201.00 unit=us scope=19999
40005,1577.80,1743.30,4585.40,4991.30,1571.80,1771.70,0.00,0.10,7735.00,8506.40,20.5,58.7,20.8,0.0
PROBE name=perf_index_subsegments metric=index_sum_p95 value=8506.40 unit=us scope=40005
ok
test app_tests::render::perf_keystroke_attribution ... # segments: core_apply_reparse=Editor::apply->Session::apply; index_rebuild=App::refresh_index（含 recompute_geometry + refresh_stats）; dirty_layout=App::ensure_block（真实缓存查询+排版）; rasterize=LayoutEngine::rasterize（脏块）; frame_compose=App::render（真实 headless 帧合成，present 无 surface 不测）
doc_blocks,core_apply_reparse_p50_us,core_apply_reparse_p95_us,index_rebuild_p50_us,index_rebuild_p95_us,dirty_layout_p50_us,dirty_layout_p95_us,rasterize_p50_us,rasterize_p95_us,frame_compose_p50_us,frame_compose_p95_us,total_p95_us,index_share_pct
2002,91.10,118.60,189.70,266.20,81.50,153.70,144.10,300.40,670.60,910.10,1749.00,15.2
PROBE name=perf_keystroke_attribution metric=total_p95 value=1749.00 unit=us scope=2002
PROBE name=perf_keystroke_attribution metric=index_share value=15.2 unit=pct scope=2002
5005,213.30,338.20,489.70,796.10,86.00,149.60,145.80,296.70,764.40,982.70,2563.30,31.1
PROBE name=perf_keystroke_attribution metric=total_p95 value=2563.30 unit=us scope=5005
PROBE name=perf_keystroke_attribution metric=index_share value=31.1 unit=pct scope=5005
10003,696.00,774.60,1619.60,1794.60,99.70,157.80,146.40,258.70,948.70,1081.20,4066.90,44.1
PROBE name=perf_keystroke_attribution metric=total_p95 value=4066.90 unit=us scope=10003
PROBE name=perf_keystroke_attribution metric=index_share value=44.1 unit=pct scope=10003
19999,1264.40,1450.10,3323.00,3593.10,111.00,172.70,148.90,271.30,1360.00,1574.70,7061.90,50.9
PROBE name=perf_keystroke_attribution metric=total_p95 value=7061.90 unit=us scope=19999
PROBE name=perf_keystroke_attribution metric=index_share value=50.9 unit=pct scope=19999
40005,2577.80,2932.80,6819.20,7296.40,131.70,177.80,155.10,273.40,2137.60,2312.20,12992.60,56.2
PROBE name=perf_keystroke_attribution metric=total_p95 value=12992.60 unit=us scope=40005
PROBE name=perf_keystroke_attribution metric=index_share value=56.2 unit=pct scope=40005
ok
test app_tests::render::perf_keystroke_path ... perf_blocks=5005
perf_keystroke_ms_p50=1.053
perf_keystroke_ms_p95=1.330
perf_keystroke_ms_max=1.466
PROBE name=perf_keystroke_path metric=keystroke_p50 value=1.053 unit=ms
PROBE name=perf_keystroke_path metric=keystroke_p95 value=1.330 unit=ms
ok
test app_tests::view::perf_after_edit_breakdown ... after_warmup_cache_layouts=18
iter0_cache_size=17
iter1_cache_size=18
iter2_cache_size=18
iter3_cache_size=18
iter4_cache_size=18
peak_total_ms=3.494
peak_refresh_index_ms=3.081
peak_ensure_layouts_visible_ms=0.092
peak_ensure_block_ms=0.001
peak_ensure_tables_visible_ms=0.001
peak_recompute_geometry_ms=0.319
peak_scroll_to_caret_ms=0.001
peak_dirty_count=1
peak_visible_range_len=14
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
p95_refresh_index_ms=1.957
p95_ensure_layouts_visible_ms=0.229
p95_recompute_geometry_ms=0.398
PROBE name=perf_after_edit_breakdown metric=peak_total_ms value=3.494 unit=ms
PROBE name=perf_after_edit_breakdown metric=p95_refresh_index_ms value=1.957 unit=ms
PROBE name=perf_after_edit_breakdown metric=p95_recompute_geometry_ms value=0.398 unit=ms
ok
test app_tests::view::perf_peak_breakdown ... peak_total_ms=16.597
peak_apply_ms=5.324
peak_after_edit_ms=1.532
peak_paint_ms=9.741
paint_spikes_gt5ms=2/500
after_edit_spikes_gt5ms=0/500
PROBE name=perf_peak_breakdown metric=peak_total_ms value=16.597 unit=ms
PROBE name=perf_peak_breakdown metric=peak_paint_ms value=9.741 unit=ms
ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1475 filtered out; finished in 32.30s

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