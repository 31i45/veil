# 阶段 D：地基加固计划

> 状态：**待确认（D1 未开始）**
> 前置：阶段 C 六片全部完成（573 测试全绿、clippy 零警告、fmt 通过）
> 来源：阶段 C 收官后的「状态模型/架构是否已到极致」评估——结论是「设计正确，但纪律仍靠人工」
> 节奏：每片独立实现、独立验收、独立 commit（用户确认后提交）、独立可 revert；每片完成后停下等确认，不擅自进入下一片。

## 1. 目标与判断标准

一句话：**把已经正确但靠人工遵守的架构纪律，变成每次 `cargo test` 自动失败的门禁。**

判断一条纪律是否真的存在，只看两条：

| 维度 | 目标 |
|---|---|
| 违规发现时延 | 一次 `cargo test` 内失败，并指出文件与期望值 |
| 回归定位时延 | 失败信息直接给出「哪条不变量、当前值、上限」 |
| 行为影响 | 阶段 D 全部切片均为「零用户可感知变化」 |

## 2. 现状基线（2026-10-26）

| 指标 | 现状 | 来源 |
|---|---|---|
| 测试 | 573 通过 / 0 失败 / 5 ignored | `cargo test --workspace` |
| 静态检查 | clippy 零警告、fmt 通过 | 阶段 C 验收 |
| `App` 顶层字段 | 10（ADR-009 目标 ≤12） | [app.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/app.rs) |
| 公共导出 | 25 项（ADR-009 目标 ≤25） | [lib.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/lib.rs) |
| 手工失效调用点 | 0 | [cache.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/view/cache.rs) |
| 依赖 | 直接 8；`Cargo.lock` 传递包 191（红线 200） | `Cargo.toml` / `Cargo.lock` |
| 性能门禁 | 无：2 条探针均 `#[ignore]` | [app_tests.rs](file:///d:/VSCode/Veil/crates/veil-egui/src/app_tests.rs#L3087-L3130) |
| CI | 无（仓库无 `.github`），全部靠手工命令 | 仓库根 |
| 属性测试覆盖 | 编辑序列、持久化状态机、render/memo 对拍；解析/HTML 未覆盖 | `app_tests.rs` |
| 最大模块（含内联测试） | app_tests 3548、editor 1419、layout 1395、highlight 1037、interaction 1031、paint 1031 行 | 行数统计 |

## 3. 切片总表

| 片 | 内容 | 关键验收 | 行为变化 |
|---|---|---|---|
| D1 | 架构不变量门禁（纯 std 测试） | 7 项检查全部有负向自测；改坏基线即红 | 零 |
| D2 | 性能基线门禁 | 真实路径 p95 有阈值断言；阈值可被触发 | 零 |
| D3 | 不可穷举输入加固 | 4 组属性测试 + 边界语料；无新增依赖 | 零 |
| D4 | 长期契约（MSRV / 公共文档 / 数字单一来源） | `missing_docs` 零警告；数字只在一处定义 | 零 |
| D5 | editor 职责接缝拆分 | 纯移动、测试数不变、生产代码 ≤700 行/文件 | 零 |
| D6 | 一键门禁 + 真实窗口冒烟 + 体量度量 | 一条命令跑完；冒烟清单 10 项可复现 | 零 |

顺序理由：D1 先行，后续切片才有机器验收；D2 次之，性能是最容易无声退化的维度；D3 补正确性；D4 定长期契约；D5/D6 才动结构，且必须先有度量。

## 4. 各片设计

### D1 架构不变量门禁

新增 `crates/veil-egui/tests/arch.rs`（只用 `std::fs` + 字符串解析，**零新依赖**），把 ADR-009 的终态指标变成可执行断言：

| # | 检查 | 上限 / 期望 |
|---|---|---|
| 1 | `App` 顶层字段数 | ≤ 12 |
| 2 | `lib.rs` 公共导出项数 | ≤ 25 |
| 3 | `view/cache.rs` 中手工失效函数（`fn invalidate_*` / `fn clear_*`） | 0 |
| 4 | `veil-egui` 直接依赖清单 | 等于基线 8 项（新增依赖必须显式改基线，评审可见） |
| 5 | `Cargo.lock` 的 `[[package]]` 数 | ≤ 200 |
| 6 | 生产源码（排除测试）中的 `unsafe` | 0 |
| 7 | `editor.rs` 的 `Intent` 分派 | 无 `_ =>` 通配臂（新意图不得被静默吞掉） |

关键要求：**每条检查函数都配一个故意违规的样例输入**，断言它会返回错误——证明门禁不是空转。验收时再逐条「改坏基线 → 测试变红 → 恢复即绿」实测一次。

验收命令：`cargo test -p veil-egui --test arch`。

### D2 性能基线门禁

- 把现有 2 条 `#[ignore]` 探针迁到 `crates/veil-egui/tests/perf.rs`，保留显式入口：
  `cargo test -p veil-egui --release --test perf -- --ignored --nocapture`
- 指标：keystroke `p50/p95`、首帧耗时；输出 `key=value` 便于对比。
- 阈值规则（先测量、后定值）：
  1. 首次运行记录基线到 `docs/perf-baseline.md`（含机器信息与日期）；
  2. 门禁阈值 = `min(基线 × 2, 16ms)`，p95 硬上限 16ms 不可放宽；
  3. 阈值收窄必须在 ADR/计划中写明理由，不允许静默调高。
- 验收：release 下跑通并写入基线；把阈值临时收紧 20% 能触发失败（证明断言生效），随后恢复。

### D3 不可穷举输入加固

复用已有 `proptest`（dev-dependency，不新增依赖），补 4 组属性：

| 组 | 属性 |
|---|---|
| 1 | 块解析↔序列化：`parse(serialize(parse(t)))` 幂等；输出区间全部为合法字符边界 |
| 2 | 行内解析：任意文本不 panic，区间合法且不越界 |
| 3 | `html_to_markdown`：任意 token 串不 panic，输出可被块解析器接受 |
| 4 | Session 随机编辑：任意 Insert/Delete/Replace 后 marks 始终落在字符边界；撤销 N 次后字节与初态完全相同 |

边界语料固定为测试常量：CRLF、BOM、1MB 单行、嵌套引用/列表、emoji ZWJ、CJK 与组合符、文末无换行。

验收：每组 ≥256 cases；失败时 proptest 自动打印最小反例；全量测试仍绿。

### D4 长期契约

- `rust-version`：用本机可用的最老稳定工具链实测最低可编译版本后写入 workspace；若旧工具链不可用，则记录当前 stable 并注明「下界未验证」，不得凭空填写。
- 公共 25 项补文档注释，启用 `#![deny(missing_docs)]`（只约束公共面，不要求内部模块）。
- 数字单一来源：`arch.rs` 中的常量（12 / 25 / 8 / 200）为唯一权威；ADR-009 与 README 中相同数字改为「以 `tests/arch.rs` 常量为准」，消除多处漂移。
- 验收：`cargo doc` 零 `missing_docs` 警告；人工核对 ADR/README 引用一致。

### D5 editor 职责接缝拆分

现状 1419 行、5 种职责：选区/marks、意图分派、缩进、列表续行、词边界。拆为：

```
editor/mod.rs    对外 API + 意图分派（生产代码 ≤700 行）
editor/word.rs   CharClass / word_left / word_right / word_run
editor/indent.rs IndentPlan / apply_indent / dedent 前缀
editor/list.rs   ListMarker / newline 续行
```

约束：纯移动，不改逻辑；`git diff -M` 应显示高相似度重命名；测试数量不变、全绿；不新增公共导出。

### D6 一键门禁与真实窗口冒烟

- `scripts/check.ps1`：依次执行 fmt check → clippy → `cargo test --workspace` → `tests/arch` → release 性能门禁；任一步失败即非零退出。
- 可选 `.github/workflows/ci.yml`（若仓库接入远端再启用；不引入任何第三方 action，只用官方 rustup/cargo）。
- `docs/manual-smoke.md`：10 项真实窗口清单——启动、打开/保存、IME 组合与候选框、拖选/双击/三击、菜单、查找/替换、粘贴图片、外部文件修改、主题切换、未保存关闭。
- 体量度量：arch 测试额外打印各模块生产代码行数（**只报告不失败**）；超过 700 行的模块记录为下一阶段拆分候选，按度量决定，不为行数而拆。

## 5. 明确不做（负空间）

- 不新增运行时依赖；不引入 `cargo-fuzz` / `libfuzzer-sys`（新依赖）。
- 不引入 ECS / 事件总线 / DI / 插件系统 / 状态管理框架（延续 ADR-009 负空间）。
- 不为行数拆文件：任何拆分必须有职责接缝与等价对拍；无度量不拆。
- 不顺手加功能、不改用户可感知行为；每片只做本片声明的检查或移动。
- 不做跨片批量重构；每片单 commit，可独立 revert。

## 6. 阶段完成定义

1. 一条命令（`scripts/check.ps1`）即可跑完全部门禁；
2. 任何违反「字段 ≤12 / 导出 ≤25 / 直接依赖 8 / 传递 ≤200 / 零手工失效 / 零 unsafe / 无通配分派」的提交，在一次 `cargo test` 内失败并给出明确数值；
3. 性能有基线文件与硬上限，退化可被捕获；
4. 解析、行内、HTML 转换、编辑序列四类不可穷举输入有属性测试与固定语料；
5. MSRV、公共文档、指标数字各有唯一来源；
6. 全部测试全绿、clippy 零警告、fmt 通过；用户可感知行为零变化。

## 7. 下一步

确认后从 **D1** 开始；D1 完成即停，等确认后再进入 D2。
