# 当前 HEAD 性能 / 内存 / 体积证据

> 日期：2026-11-26
> HEAD：`5b3d9a6`（Phase 1 完成后）
> 版本：`3.0.2`；工具链：rustc/cargo 1.99.1
> 机器：Intel Core i9-14900HX / 31.7 GB RAM / Windows 11 企业版 10.0.26300
> 构建：release（lto=true, codegen-units=1, panic=abort, strip=true, opt-level="s"）

## 1. 复现

```powershell
cargo build --release --bin veil
pwsh -NoProfile -File tools/verify/perf_gate.ps1
```

原始输出：[perf-gate-2026-11-26.md](perf-gate-2026-11-26.md)
原始样本：[bench.json](raw/bench.json)、[bench_typing.txt](raw/bench_typing.txt)、[keystroke_p99.txt](raw/keystroke_p99.txt)

## 2. 结果

| 项 | 实测 | 阈值 | 口径 | 判定 |
|---|---:|---:|---|---|
| 5005 块全量排版 | **296.55 ms** | < 100 ms | `Session::open` + 每块 `layout_block` 一次，5 次中位；非端到端首帧；128535 字符 | **FAIL** |
| 同一轮全量排版最大值 | 333.33 ms | — | 同上 5 次最大值 | 参考 |
| 击键 p99（内核 apply） | **0.305 ms** | < 16 ms | 5005 块 / 1000 次单字符 `Session::apply` / 最近秩 p99 | PASS |
| 击键 p50 / p95（内核 apply） | 0.073 / **0.109 ms** | — | 同上 1000 样本 | 参考 |
| 击键峰值 max | 0.641 ms | — | 同上 | 参考 |
| 端到端首帧 | **84.8 ms**（中位） | < 100 ms | 真实 `veil.exe`（`VEIL_PERF_TRACE=1`）打开 101287 字符文档；5 次：74.4 / 76.8 / 84.8 / 87.6 / 109.6 ms | PASS |
| 常驻内存峰值 RSS | **52.6 MB** | < 300 MB | 真实进程 10 s / 100 ms 采样 WorkingSet64 | PASS |
| Private Bytes 峰值 | 33.76 MB | — | 同轮参考 | 参考 |
| 二进制体积 | **14,170,112 B** | ≤ 14,680,064 B | `target\release\veil.exe`；基线 14,135,296 B | PASS |
| SHA-256 | 见下方 | — | `Get-FileHash` | 记录 |

## 3. 关键结论

1. **端到端首帧、击键、RSS、体积四项通过**；用户打开约 10 万字文档时，首帧 84.8 ms，击键内核 p95 0.109 ms，RSS 52.6 MB。
2. **5005 块全量排版 FAIL**：296.55 ms，超 100 ms 阈值 196.55 ms。它与“端到端首帧”不是同一口径；不能用首帧 84.8 ms 掩盖这项失败。
3. 这项 FAIL 说明：**报告 ⑤ 的 4/5 不能再无条件引用**。当前版本“感知启动快”成立；“全量排版 <100ms”不成立。Phase 6 评估必须分开写。
4. 二进制体积距 14 MiB 门禁仅约 509,952 B 余量；后续任何依赖新增都要先称重。

## 4. SHA-256

```text
73A08EE9F0260AEFEA2C99ECC7BEB9EEF476E804E1908900362E4EFB05A5A3C3
```

## 5. 局限

- 击键样本是合成语料，不是中文 IME 真实输入；IME 证据另见 `docs/review/evidence/ux/`。
- 端到端首帧终点是 softbuffer `present` 成功，早于窗口合成器真正上屏；严格说是“首帧呈现就绪”。
- 全量排版是内部门禁，不是用户直接感知指标；但它能解释大文档滚动/重排的余量。
