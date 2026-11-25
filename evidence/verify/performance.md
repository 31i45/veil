# Veil 性能门禁（本轮实测）

本文件是「文档声称」到「每轮可核对的表」的落地：数字全部由本机真实执行产生，
脚本为 `tools/verify/perf_gate.ps1`（调用 `bench/run_bench.ps1` 与真实进程内存采样）。
自动区块之外的手写说明只解释口径与边界，不重复放数字。

## 本轮改动与口径约定

- 本轮影响性能的产品改动：新增源码模式纯文本行视图（`source_plain_lines`，默认关闭），
  不改变本文件的四项门禁口径。
- **两个「首帧」必须分开**：
  - 「5005 块全量排版」= 进程内 `Session::open` 后把全部 5005 块各排版一次的中位耗时
    （`bench_typing.cold_start_ms`），是下面 100 ms 门禁的口径；
  - 「端到端首帧」= 进程创建到像素真正上屏，需要屏幕级时间戳采样，本轮**不可测**，
    因此没有被拿来当门槛，也没有用「冷启动到窗口句柄」冒充。
- 文档里出现过的 553–611 ms 是**冷启动（进程创建 → 窗口句柄出现）**，既不是端到端首帧，
  也不是 5005 块全量排版，引用时必须带这个限定语。
- 击键门禁的 p99 来自内核 `Session::apply` 段（与既有 `bench_typing` 同口径、补 p99）；
  真实视图全路径的既有探针只输出 p50/p95/max，未输出 p99，本文件不虚构全路径 p99。
- 常驻内存用**峰值 RSS（WorkingSet64）**，Private（私有提交）只作参考。
- 二进制体积取 `target\release\veil.exe` 实际字节，阈值 14,680,064 B、基线 14,135,296 B 固定不变。

## 复现

```powershell
cargo build --release --bin veil
pwsh -NoProfile -File tools/verify/perf_gate.ps1              # 四项门禁 + 证据表
pwsh -NoProfile -File tools/verify/perf_gate.ps1 -WithProbe   # 额外跑既有全路径击键探针
pwsh -NoProfile -File bench/run_bench.ps1                     # 只看基准数字
```

<!-- perf-gate:begin（本区块由 tools/verify/perf_gate.ps1 生成，重复运行整体替换） -->
## 自动门禁结果

- 时间：2026-11-25 15:58:50 +08:00
- 机器：Intel(R) Core(TM) i9-14900HX / 31.7 GB RAM / Microsoft Windows 11 企业版 10.0.26300
- 工具链：rustc 1.99.1 (58cf8f9c3 2026-10-13)
- 构建类型：release（profile：lto = true, codegen-units = 1, panic = "abort", strip = true, opt-level = "s"）
- 二进制：`target\release\veil.exe` 14142976 B（基线 14135296 B）
- 运行命令：`pwsh -NoProfile -File tools/verify/perf_gate.ps1`

### 四项门禁

| 项 | 实测 | 阈值 | 口径 | 判定 | 差额 |
|---|---|---|---|---|---|
| 10 万字文档（5005 块）全量排版 | 586.68 ms | < 100 ms | Session::open + 每块 layout_block 一次，5 次中位（bench_typing.cold_start_ms）；非端到端首帧；文档 5005 块 / 128535 字符 | **FAIL** | 超出 486.68 ms |
| (参考) 同一轮 5005 块全量排版最大值 | 590.89 ms | — | 同一基准 5 次的最大值 | 参考 | 参考值，不参与判定 |
| 击键 p99 | 0.303 ms | < 16 ms | 5005 块 / 1000 次单字符 Session::apply / 最近秩 p99（bench/keystroke_p99.rs，同 bench_typing 口径） | PASS | 余量 15.697 ms |
| (参考) 击键 p50 / p95 | 0.068 / 0.111 ms | — | 同基准核内 apply 段 | 参考 | 参考值，不参与判定 |
| (参考) 击键峰值 max | 0.998 ms | — | 同基准 1000 样本最大 | 参考 | 参考值，不参与判定 |
| 常驻内存（峰值 RSS） | 45.94 MB | < 300 MB | 真实 veil.exe 打开 101287 字符文档，WorkingSet64 10 s 峰值 | PASS | 余量 254.06 MB |
| (参考) 冷启动到窗口句柄 | 145 ms | — | 进程创建 → MainWindowHandle 出现；单次测量、波动大；**不是端到端首帧** | 参考 | 参考值，不参与判定 |
| 二进制体积 | 14142976 B | ≤ 14,680,064 B | target\release\veil.exe 实际字节；基线 14,135,296 B | PASS | 余量 537088 B（相对基线 7680 B） |

### 参考数字（不参与判定）

- 既有全路径探针 perf_keystroke_path（apply + after_edit + paint，300 样本）：p50 1.308 ms / p95 1.864 ms / max 2.193 ms，块数 5005；该探针只输出 p50/p95/max，不输出 p99。
- 同轮的脏块重排 p50/p95：263.9 / 503.1 µs；帧分配 p50 1.75 ms vs 复用 p50 0.42 ms；存活堆峰值 6.25 MB。
- 5005 块基准文档体量（同一构造，实测）：128535 字符。
- 内存采样：10 s / 100 次（每 100 ms），最终 Responding=True；Private 峰值 27.36 MB（参考，非判定口径）。

### 不可测项

- 无

### 口径辨析（必读）

- **「5005 块全量排版」不是「端到端首帧」**：前者是进程内 Session::open 后把全部 5005 个块各排版一次的中位耗时，
  是 bench_typing 既有基准的冷启动口径；后者是进程创建到像素真正上屏，本脚本没有屏幕像素时间戳采样，
  **不测、也不拿别的东西冒充**。上表「冷启动到窗口句柄」这一项同样不是端到端首帧。
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
