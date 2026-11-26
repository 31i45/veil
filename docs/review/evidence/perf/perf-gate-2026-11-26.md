# Veil 性能门禁（本轮实测）

本文件是「文档声称」到「每轮可核对的表」的落地：数字全部由本机真实执行产生，
脚本为 `tools/verify/perf_gate.ps1`（调用 `bench/run_bench.ps1`、真实进程内存采样与首帧埋点）。

## 本轮改动与口径约定

- 本轮影响性能的产品改动：新增源码模式纯文本行视图（`source_plain_lines`，默认关闭），
  不改变本文件的门禁口径。
- 首帧埋点：`crates/veil-ui/src/diag.rs` 提供 `perf_mark_start` / `perf_trace_first_frame`
  （只在 `VEIL_PERF_TRACE=1` 时读时钟并打印 `veil-first-frame-ms: <ms>`，每进程一次），
  计时原点在 `crates/veil/src/lib.rs::run()` 首行；`app.rs` 首帧状态机仅在**首次 present 成功**
  时调用埋点。未开启该环境变量时不打印、不做 IO，正常启动路径只多一次原子检查。
- **两个「首帧」必须分开**：
  - 「5005 块全量排版」= 进程内 `Session::open` 后把全部 5005 块各排版一次的中位耗时
    （`bench_typing.cold_start_ms`），不含进程启动、窗口创建与 present；
  - 「端到端首帧」= 进程入口 `veil::run()` 首行 → 首次 `present` 成功（`VEIL_PERF_TRACE=1`
    埋点，release veil.exe + 约 10 万字文档，5 次取中位）。
- 端到端首帧的终点是 softbuffer `present` 返回成功，**早于窗口合成器真正上屏**，
  因此严格说是「首帧呈现就绪」而不是屏幕像素时间戳；但它确实覆盖进程启动到首帧呈现，
  不是「冷启动到窗口句柄」，也没有被冷启动数字顶替。
- 文档里出现过的 553–611 ms 是**冷启动（进程创建 → 窗口句柄出现）**，
  既不是端到端首帧，也不是 5005 块全量排版，引用时必须带这个限定语。
- 击键门禁的 p99 来自内核 `Session::apply` 段（与既有 `bench_typing` 同口径、补 p99）；
  真实视图全路径的既有探针只输出 p50/p95/max，未输出 p99，本文件不虚构全路径 p99。
- 常驻内存用**峰值 RSS（WorkingSet64）**，Private（私有提交）只作参考。
- 二进制体积取 `target\release\veil.exe` 实际字节，阈值 14,680,064 B、基线 14,135,296 B 固定不变。

## 复现

```powershell
cargo build --release --bin veil
pwsh -NoProfile -File tools/verify/perf_gate.ps1              # 全部门禁 + 证据表
pwsh -NoProfile -File tools/verify/perf_gate.ps1 -WithProbe   # 额外跑既有全路径击键探针
pwsh -NoProfile -File bench/run_bench.ps1                     # 只看基准数字
```

<!-- perf-gate:begin（本区块由 tools/verify/perf_gate.ps1 生成，重复运行整体替换） -->
## 自动门禁结果

- 时间：2026-11-26 12:49:59 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / 31.7 GB RAM / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 构建类型：release（profile：lto = true, codegen-units = 1, panic = "abort", strip = true, opt-level = "s"）
- 二进制：`target\release\veil.exe` 14170112 B（基线 14135296 B）
- 运行命令：`pwsh -NoProfile -File tools/verify/perf_gate.ps1`

### 门禁项

| 项 | 实测 | 阈值 | 口径 | 判定 | 差额 |
|---|---|---|---|---|---|
| 10 万字文档（5005 块）全量排版 | 296.55 ms | < 100 ms | Session::open + 每块 layout_block 一次，5 次中位（bench_typing.cold_start_ms）；非端到端首帧；文档 5005 块 / 128535 字符 | **FAIL** | 超出 196.55 ms |
| (参考) 同一轮 5005 块全量排版最大值 | 333.33 ms | — | 同一基准 5 次的最大值 | 参考 | 参考值，不参与判定 |
| 击键 p99 | 0.305 ms | < 16 ms | 5005 块 / 1000 次单字符 Session::apply / 最近秩 p99（bench/keystroke_p99.rs，同 bench_typing 口径） | PASS | 余量 15.695 ms |
| (参考) 击键 p50 / p95 | 0.073 / 0.109 ms | — | 同基准核内 apply 段 | 参考 | 参考值，不参与判定 |
| (参考) 击键峰值 max | 0.641 ms | — | 同基准 1000 样本最大 | 参考 | 参考值，不参与判定 |
| 端到端首帧（进程启动 → 首次 present） | 84.8 ms（5 次中位） | < 100 ms | 真实 veil.exe（VEIL_PERF_TRACE=1）打开 101287 字符文档；5 次：74.4 / 76.8 / 84.8 / 87.6 / 109.6 ms；起点 run() 首行，终点首次 present 成功 | PASS | 余量 15.2 ms |
| 常驻内存（峰值 RSS） | 52.6 MB | < 300 MB | 真实 veil.exe 打开 101287 字符文档，WorkingSet64 10 s 峰值 | PASS | 余量 247.4 MB |
| 二进制体积 | 14170112 B | ≤ 14,680,064 B | target\release\veil.exe 实际字节；基线 14,135,296 B | PASS | 余量 509952 B（相对基线 34816 B） |

### 参考数字（不参与判定）

- 既有全路径探针 `perf_keystroke_path` 本轮未运行（加 `-WithProbe` 可运行；它只输出 p50/p95/max，无 p99）。
- 同轮的脏块重排 p50/p95：278.5 / 479.3 µs；帧分配 p50 1.84 ms vs 复用 p50 0.46 ms；存活堆峰值 74.34 MB。
- 5005 块基准文档体量（同一构造，实测）：128535 字符。
- 内存采样：首帧第 1 次运行之后，10 s / 100 次（每 100 ms），最终 Responding=True；Private 峰值 33.76 MB（参考，非判定口径）。

### 不可测项

- 无

### 口径辨析（必读）

- **两项「首帧」分开列，不可混用**：
  - 「5005 块全量排版」= 进程内 Session::open 后把全部 5005 块各排版一次的中位耗时（bench_typing.cold_start_ms），
    **不含进程启动、窗口创建与 present**；
  - 「端到端首帧」= 进程入口 `veil::run()` 首行 → 首次 present 成功的毫秒数（`VEIL_PERF_TRACE=1` 埋点，
    真实 release veil.exe、约 10 万字文档、5 次中位）。它覆盖进程启动、CLI/config、窗口创建、文档解析、
    首帧排版、光栅与 present，终点是 softbuffer `present` 返回成功，**早于窗口合成器真正上屏**，
    因此严格说是「首帧呈现就绪」，不是屏幕像素时间戳；但它也不是冷启动到窗口句柄，且**没有**被拿冷启动数字顶替。
- **击键 p99 的口径是内核 apply 段**（5005 块、1000 次单字符插入、最近秩 p99），与 bench_typing 同源；
  真实视图全路径（apply + refresh_index + 脏块排版 + 光栅 + 帧合成）的既有探针只输出 p50/p95/max，
  未加 `-WithProbe` 时则没有全路径数字，且该探针**没有 p99**。
- **常驻内存用峰值 RSS（WorkingSet64）**：真实 veil.exe 打开约 10 万字文档后每 100 ms 采样 10 s 取峰值；
  Private（私有提交）仅作参考，不是判定口径。
- 体积以 `target\release\veil.exe` 实际字节为准，阈值 14,680,064 B 与基线 14,135,296 B 均为固定常量。

### 本次判定

- 结果：**FAIL**（见上表 FAIL/不可测项）
- 脚本退出码：1
<!-- perf-gate:end -->
