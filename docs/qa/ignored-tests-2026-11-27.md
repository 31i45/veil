# ignored 测试清单（2026-11-27）

> 范围：`cargo test --workspace` 里全部 `#[ignore]` 测试。本清单以**源码为准**逐条核对，
> 与任务书提供的 17 条清单存在行号/名称差异，已在文末「与任务书清单的差异」注明。
> 门禁：`pwsh -NoProfile -File tools/verify/probe-gate.ps1`（release 构建、逐条 `--ignored --exact --nocapture`）。
> 不是 README 的「已知限制」清单：这里只登记测试探针，不代表产品功能缺失。

## 汇总

| 类型 | 条数 | 说明 |
|---|---|---|
| 性能 | 11 | 只有 release 优化级别下的数字有参考意义，保留 `#[ignore]`，由 probe-gate 覆盖 |
| 病态 | 4 | 病态输入/超大单块：要求不 panic、不挂起、往返一致；保留 `#[ignore]` 以避开 debug 耗时与内存压力 |
| 诊断 | 1 | `mindmap_viewbox_debug` 已**晋级**为常规断言（不再 ignore），详见证件 |
| 证据 | 1 | CJK PDF 证据生成：依赖真实系统字体，保留 `#[ignore]` 但改为 probe-gate **必跑**，缺字体记 SKIP |

当前 `#[ignore]` 计数：**16**（17 条原始条目中 `mindmap_viewbox_debug` 已晋级）。
`pathological_eval_child` 在门禁中以 `loop` / `microtask` 两种模式各运行一次，因此 probe-gate 的判定行数为 18。

## 逐条清单

| # | 测试名 | 文件:行 | 类型 | 为什么 ignore | 运行条件 | 判定方式（指标 / 阈值 / PASS 条件） | 门禁覆盖 |
|---|---|---|---|---|---|---|---|
| 1 | `pathological_eval_child` | `crates/veil-mermaid-offline/src/lib.rs:745`（fn :746） | 病态 | 子进程探针，正常由 `pathological_single_eval_has_a_real_deadline` 以环境变量拉起；单独运行无意义 | `VEIL_PATHOLOGICAL_EVAL=loop\|microtask`，release 或 debug 均可 | 退出码 0；`PROBE answer=42`（200 ms deadline 中断后运行时仍可用）；300 s 内结束，超时即 FAIL | probe-gate 必跑（两种模式） |
| 2 | `mindmap_viewbox_debug` | `crates/veil-mermaid-offline/src/lib.rs:2489` | 诊断 | **已晋级**：原为「只在显式运行写临时目录」的诊断器，现改为常规断言 | `cargo test --workspace` 直接执行；`VEIL_MINDMAP_DUMP=1` 时才额外写临时 SVG | SVG 可解析；`viewBox` 存在、宽高 ≥ 16、宽高比 ≤ 20（复用 `svg_view_box`）；`PROBE viewbox_width/height > 0` | 常规 `cargo test` + probe-gate 复核 |
| 3 | `bench_edit_reparse_history_100k` | `crates/veil-core/src/session.rs:739`（fn :740） | 性能 | 10 万字文档 50 次编辑的微基准，debug 下数字无意义 | `cargo test -p veil-core --release --lib -- --ignored` | `PROBE reparse_us` / `edit_us` 的 p95 ≤ 基线 p95 × 1.2；另断言块数 ≥ 2000 | probe-gate 必跑 |
| 4 | `pathological_inputs_matrix` | `crates/veil-core/tests/pathological.rs:76`（fn :77） | 病态 | 10 类病态输入（10MB 单行、12 万块、5 千层括号等），release 下耗时才有参考意义；debug 运行慢且吃内存 | `cargo test -p veil-core --release --test pathological -- --ignored` | 退出码 0（无 panic、不挂起）；每类往返一致；`PROBE cases_passed=10`、`max_ms`；900 s 超时 | probe-gate 必跑 |
| 5 | `perf_50mb_long_doc` | `crates/veil-ui/src/app_tests/persist.rs:2067`（fn :2068） | 性能 | 50 MB 中文长文探针，内存/耗时压力大，不进常规门禁 | `cargo test -p veil-ui --release --lib -- --ignored`；建议机器空闲、≥16 GB 内存 | 探针自身断言：打开 ≤ 5 000 ms、击键 p95 ≤ 16 ms、保存字节与会话序列化逐字节一致；门禁另按 `PROBE open_ms` / `keystroke_p95` 的预算与基线 p95×1.2 判定。内存分配失败记 SKIP（不得记 PASS） | probe-gate 必跑 |
| 6 | `perf_20k_blocks` | `crates/veil-ui/src/app_tests/persist.rs:2152`（fn :2153） | 性能 | 约 21k 块文档 300 次真实击键（apply + after_edit + paint），仅 release 有意义 | 同上 | 断言 300 次击键零丢字、保存字节一致；`PROBE keystroke_p95` ≤ 基线 p95 × 1.2 | probe-gate 必跑 |
| 7 | `perf_keystroke_path` | `crates/veil-ui/src/app_tests/render.rs:1814`（fn :1815） | 性能 | 真实视图全路径（apply + after_edit + paint），release 下才有参考价值 | `cargo test -p veil-ui --release --lib -- --ignored` | `PROBE keystroke_p95` ≤ 基线 p95 × 1.2；同时记录 p50 | probe-gate 必跑 |
| 8 | `perf_keystroke_attribution` | `crates/veil-ui/src/app_tests/render.rs:1859`（fn :1860） | 性能 | 5 档规模 × 300 样本的分段归因，运行时间长 | 同上 | 每档 `PROBE total_p95`（scope=块数）≤ 对应基线 × 1.2；`index_share` 仅记录 | probe-gate 必跑 |
| 9 | `perf_index_subsegments` | `crates/veil-ui/src/app_tests/render.rs:1979`（fn :1980） | 性能 | `refresh_index` 4 个子段 × 5 档规模，release 专用 | 同上 | 每档 `PROBE index_sum_p95` ≤ 基线 × 1.2 | probe-gate 必跑 |
| 10 | `perf_cold_first_edit` | `crates/veil-ui/src/app_tests/render.rs:2069`（fn :2070） | 性能 | 冷启动「构造 + 首编辑」观测项，受 OS 文件缓存影响大 | 同上 | `PROBE cold_first_edit_ms` ≤ 基线 p95 × 1.2；多采样用 `-Repeat` | probe-gate 必跑 |
| 11 | `perf_cold_first_edit_prewarmed` | `crates/veil-ui/src/app_tests/render.rs:2096`（fn :2097） | 性能 | 对照组：预热字形后的首编辑成本，需与 #10 同口径 | 同上 | `PROBE edit_ms` ≤ 基线 p95 × 1.2 | probe-gate 必跑 |
| 12 | `perf_first_edit_after_first_frame` | `crates/veil-ui/src/app_tests/render.rs:2116`（fn :2117） | 性能 | 真实生命周期（构造→首帧→首键），release 专用 | 同上 | `PROBE first_edit_ms` ≤ 基线 p95 × 1.2；`first_frame_ms` 仅记录 | probe-gate 必跑 |
| 13 | `perf_huge_scroll` | `crates/veil-ui/src/app_tests/render.rs:2144`（fn :2145） | 病态 | 10 MB 单块连续滚轮 300 格，内存压力大 | 同上 | 断言滚到末尾仍有内容（`nonbg > 1000`）；`PROBE nonbg` 复核 ≥ 1001；不建性能基线 | probe-gate 必跑 |
| 14 | `perf_huge_block` | `crates/veil-ui/src/app_tests/render.rs:2179`（fn :2180） | 病态 | 1/2/10 MB 单块只排前缀的保护验证，3 档合计内存占用高 | 同上 | 退出码 0（断言 `truncated`、排版文本 ≤ `MAX_BLOCK_BYTES`、序列化长度一致）；`PROBE app_ms/edit_ms/tail_ms`（按 MB 分档）仅记录 | probe-gate 必跑 |
| 15 | `perf_peak_breakdown` | `crates/veil-ui/src/app_tests/view.rs:2305`（fn :2306） | 性能 | 500 帧峰值定位（apply / after_edit / paint 拆分），release 专用 | 同上 | `PROBE peak_total_ms` ≤ 基线 p95 × 1.2；`peak_paint_ms` 仅记录 | probe-gate 必跑 |
| 16 | `perf_after_edit_breakdown` | `crates/veil-ui/src/app_tests/view.rs:2366`（fn :2367） | 性能 | `after_edit` 6 个子步骤拆分，500 次迭代 | 同上 | `PROBE peak_total_ms`、`p95_refresh_index_ms` ≤ 各自基线 × 1.2 | probe-gate 必跑 |
| 17 | `generate_wave4_sample_pdf` | `crates/veil/src/platform/pdf_layout.rs:4247`（fn :4248） | 证据 | 需要真实系统 CJK 字体与 `.trae/specs/wave4-pdf-fidelity/evidence/sample.md`，非 CI 通用环境 | Windows；系统存在 CJK 字体；`cargo test -p veil --lib -- --ignored` | 退出码 0；`target/wave4-sample.pdf` 存在且非空；`PROBE pages ≥ 3`、`bytes > 0`、`outline ≥ 1`；缺字体记 SKIP 并写明原因 | probe-gate 必跑（缺字体 SKIP） |

## 晋级与保留

- **晋级（移出 `#[ignore]`）**：`mindmap_viewbox_debug`。原诊断器改为常规断言（viewBox 可解析、尺寸合理），
  临时 SVG 落盘改为 `VEIL_MINDMAP_DUMP=1` 显式开关，避免常规测试写临时目录。
- **门禁必跑但保留 ignore**：`generate_wave4_sample_pdf` —— 依赖系统字体，常规 `cargo test` 在无字体环境会假失败；
  改由 probe-gate 运行，字体缺失时明确 SKIP。
- **保留 ignore 且门禁覆盖**：其余 15 条性能/病态探针。原因见上表；probe-gate 逐条按精确测试名执行，
  失败或超时非零退出。
- `pathological_eval_child` 是子进程机制，常规门禁由未 ignore 的
  `pathological_single_eval_has_a_real_deadline` 拉起；probe-gate 另以两种模式直接执行以防“父测试被跳过”时无人发现。

## 机器可读指标

上述探针新增一行（或多行）`PROBE name=<name> metric=<metric> value=<number> unit=<unit> [scope=<scope>]`。
只新增输出，未改动任何既有预算或断言逻辑。probe-gate 解析这些行；没有 PROBE 行或指标缺失即 FAIL。

## 本轮实测（2026-11-27，本机 i9-14900HX / Windows 11 26300 / rustc 1.99.1）

- 首次运行：`cargo test --workspace --release --no-run` 构建耗时 **185.5 s**，18 条判定共 **300 s**（含首次加载大体积测试二进制），PASS 18 / FAIL 0 / SKIP 0。
- 复跑（二进制已构建，`-UpdateBaseline`）：构建 2.1 s，探针 **52.2 s**，PASS 18 / FAIL 0 / SKIP 0；基线 22 项写入 `docs/qa/perf-baseline.json`。
- 明细（每条状态、耗时、min/p50/p95/max 指标）见 `docs/qa/probe-gate-2026-11-27.md`。
- 无 FAIL、无 SKIP：CJK 证据生成本机命中系统字体，产出 `target/wave4-sample.pdf`（252,279 B、4 页、14 条书签）。

## 与任务书清单的差异（以源码为准）

| 任务书条目 | 源码实际 | 说明 |
|---|---|---|
| `crates/veil-mermaid-offline/src/lib.rs:743` | `:745`（`fn` :746） | 行号因文档注释/前序编辑偏移 |
| `.../lib.rs:2389` `mindmap_viewbox_debug` | `:2489` | 行号偏移；且该测试已晋级，不再 ignore |
| `crates/veil-core/src/session.rs:739` `bench_edit_reparse_history` | `bench_edit_reparse_history_100k` | 函数名以源码为准 |
| `crates/veil-core/tests/pathological.rs:76`「病态输入探针」 | `pathological_inputs_matrix` | 未给函数名，源码名为 `pathological_inputs_matrix` |
| `crates/veil/src/platform/pdf_layout.rs:4084` | `:4247`（`fn` :4248） | 行号偏移 |
