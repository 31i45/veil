# ADR-014: Quick Open 保持手写模糊匹配 —— 不引入 fuzzy-matcher/nucleo

## 状态

已采纳（2026-11-17）。依据：`docs/review/component-replacement-spike.md` S5。

## 背景

Quick Open 的候选过滤与排序由 `crates/veil-ui/src/view/quick_open.rs` 手写 `score()`/`filter()`（约 35 行）：查询字符按序命中即匹配，连续匹配 +12、词首 +8、按文本长度扣分，最近文件同分 +10。规格提出用 `fuzzy-rapid` 替换。

**事实核查：crates.io 上不存在 `fuzzy-rapid`**（`cargo info fuzzy-rapid` 报 `could not find`）。因此评估两个最接近的成熟替代：`fuzzy-matcher` 0.3.7（MIT，skim 算法）与 `nucleo` 0.5.0（MPL-2.0，Helix 引擎；异步流式设计，对 ≤500 条候选过度设计）。

## 实测

探针：243 条候选（20 目录 × 12 文件名 + 3 个最近文件），Veil 算法原样复制；查询 `rust` / `python` / `config`。`precision@10` 按「同名去重后的前 10 条中与查询明确相关的文件名数」计。

| 查询 | Veil | fuzzy-matcher (SkimV2) |
|---|---|---|
| `rust` | 4/10 | 4/10 |
| `python` | 4/10 | 4/10 |
| `config` | 5/10 | 2/10 |

| 指标 | Veil 手写 | fuzzy-matcher |
|---|---|---|
| 单次过滤（243 候选） | 144.8 µs | 73.2 µs |
| 500 候选折算 | ≈0.30 ms | ≈0.15 ms |
| exe 增量 | 0 | +92,160 B（余量 11,232 B 的 8.2 倍） |
| 新增 unique 包 | 0 | 2（`fuzzy-matcher` 本身 + `thread_local`） |

## 决策

保持手写实现，不引入模糊匹配依赖。`fuzzy-rapid` 因不存在而不可用；`fuzzy-matcher` / `nucleo` 均不采纳。

## 理由

1. **目标组件不存在**：`fuzzy-rapid` 无可替换对象。
2. **质量无净收益**：`config` 查询下 Skim 前排精度更低（2/10 vs 5/10），`rust`/`python` 持平。手写的词首/连续/长度惩罚 + 最近文件加成贴合文件路径候选。
3. **速度差异无感**：0.30 ms vs 0.15 ms 都在击键路径噪声内，远低于 16 ms 门禁。
4. **体积一票否决**：+92,160 B 对 11,232 B 余量。

## 后果

- Quick Open 排序继续自研；新增需求（如路径加权、frecency）在现有 `score()` 内演进。
- 若未来排序质量成为实际痛点，可在手写实现内引入 Skim 的「最优子序列 + 连续/词首权重」动态规划，属纯代码改进、零体积代价；不构成组件替换。
- `CANDIDATE_CAP = 500` 的上限继续保证最坏路径耗时 < 0.5 ms。

## 否决的替代方案

| 方案 | 结论 | 理由 |
|---|---|---|
| fuzzy-rapid | 不可用 | crates.io 无此包 |
| fuzzy-matcher 0.3.7 | 否决 | 精度无净收益，+92 KB 超余量 8 倍 |
| nucleo 0.5.0 | 否决 | MPL-2.0、异步流式引擎，对 ≤500 候选过度设计，依赖与体积更大 |
