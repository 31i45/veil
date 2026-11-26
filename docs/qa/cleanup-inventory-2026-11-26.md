# 仓库清理盘点（2026-11-26）

> 目的：为「干净可复现的仓库基线」提供只读盘点证据。本文档**不执行**任何删除，删除动作见 §6。
> 范围：仅盘点未跟踪/被忽略的构建产物、日志与临时文件，以及经引用检查确认的私有死代码。
> 约束：不删除已跟踪文件、`docs/review`、`.trae`、测试夹具、feature-gated 代码与公开 API。

## 1. 基线快照

| 项 | 值 |
|---|---|
| 分支 / 最近提交 | `6032a40 test(recovery): 新增 4 个恢复路径回归门禁` |
| `git status --porcelain` | ` M crates/veil/examples/bench_typing.rs`（工作区原有改动；`git diff` 内容为空，按要求不回滚、不覆盖） |
| 未跟踪且未被忽略的文件 | 0（`git ls-files --others --exclude-standard` 为空） |
| 被忽略条目总数 | 18,991（其中 target 14,920） |

仓库根目录清单（顶层）：

```
.git/  .github/  .screenshots/  .skills/  .trae/  bench/  crates/  docs/  evidence/
icons/  installer/  release-bundle/  target/  tools/
.gitattributes  .gitignore  Cargo.lock  Cargo.toml  CHANGELOG.md  deny.toml  LICENSE
PRIVACY.md  README.md  README.zh-CN.md  rust-toolchain.toml  SECURITY.md
SUPPORT.md  THIRD-PARTY-LICENSES.md
```

## 2. 磁盘占用（清理前）

| 目录 | 文件数 | 体积 |
|---|---|---|
| `target/` | 14,920 | **22.86 GB** |
| `.trae/`（本地 agent 工作区，忽略） | 1,913 | 527.9 MB |
| `evidence/`（真机证据包，忽略） | 2,288 | 348.0 MB |
| `release-bundle/`（忽略） | 20 | 43.4 MB |
| `.git/` | 1,041 | 14.4 MB |
| `crates/`（源码） | 1,547 | 8.5 MB |
| `docs/` | 101 | 2.9 MB |
| `bench/`（语料 + 结果） | 57 | <0.1 MB |

D 盘剩余空间：约 411 GiB。`target/` 是本仓库唯一的大体量可再生产物。

## 3. 未跟踪 / 被忽略产物清单

按顶层目录聚合（`git status --porcelain --ignored -uall`）：

| 类别 | 条目数 | 性质 | 本次处置 |
|---|---|---|---|
| `target/` | 14,920 | Cargo 构建产物，`cargo clean` 可完全再生 | **删除** |
| `evidence/` | 2,146 | 真机证据包（截图/日志，`.gitignore` 明确本地留存） | 保留（证据资产） |
| `.trae/` | 1,913 | agent 规格与证据（spec/checklist/日志），约束禁止删除 | 保留 |
| `bench/results/` | 55 | 基准运行结果，`tools/bench/*` 可再生 | 保留（不在本次授权范围） |
| `docs/review/evidence/` | 21 | 评审截图（PNG）+ 当日采集日志与 perf raw | 保留（`docs/review` 受约束保护） |
| `release-bundle/` | 20 | 发布包产物 | 保留（不在本次授权范围） |
| `.skills/` | 7 | 本地技能实例化 | 保留 |

细项核查：

- 仓库根目录：无 `*.log` / `*.tmp` / 临时导出 `*.html` / `*.pdf` / `Thumbs.db` / `.DS_Store`（实查为空）。
- `docs/` 递归：仅 2 个 `*.log`，均位于 `docs/review/evidence/ux/`：
  - `docs/review/evidence/ux/high-contrast-2026-11-26.log`（1.7 KB，2026-11-26 12:50）
  - `docs/review/evidence/ux/scale-capture-2026-11-26.log`（4.2 KB，2026-11-26 12:50）
  两者是当日其他代理正在产出的评审证据；`docs/review` 属保护目录，**保留**。
- `*.tmp` / `Thumbs.db` / `.DS_Store` / `*.orig` / `*.rej`：仓库内（排除 `target`、`.git`）未发现。
- 说明：`bench/results/`、`evidence/`、`docs/review/evidence/**/*.png` 均被 `.gitignore` 明确视为本地留存，不属于「构建产物/日志/临时文件」，本次不动。

## 4. 死代码候选

`cargo build --workspace`（含强制重检 `cargo check --workspace --all-targets`）当前 **零告警**；原因是代码库广泛使用 `#[allow(dead_code)]` 抑制。逐项核验如下（引用检查基于 rg 全仓搜索）：

| 位置 | 符号 | 引用检查结论 | 处置 |
|---|---|---|---|
| `crates/veil-mermaid-offline/src/lib.rs:107` | `Session.rt` | 字段顺序即析构顺序（Context 必须先于 Runtime 释放），生命周期必需 | 保留 |
| `crates/veil-mermaid-offline/src/lib.rs:282` | `Session::query_canvas_trace`（私有） | 全仓 0 调用；仅 `assets/bootstrap.js` 保留同名 JS 调试数组 | **删除** |
| `crates/veil-mermaid-offline/src/lib.rs:301` | `Session::query_canvas_svg`（私有） | 全仓 0 调用；同上 | **删除** |
| `crates/veil/src/platform/windows.rs:28` | `OpenFileNameW` | Win32 FFI 布局结构体，部分字段仅写入不读出，`allow` 属必要 | 保留 |
| `crates/veil/src/platform/assoc.rs:14` | `#![cfg_attr(not(windows), allow(dead_code))]` | 平台 cfg 分支 | 保留 |
| `crates/veil-ui/src/shell.rs:18` | `product_error` 模块导入 | `cfg(all(test, windows))` 测试目标专用 | 保留 |
| `crates/veil-ui/src/autosave.rs:32` | 默认 idle 时长 | `cfg(test)` 断言用 | 保留 |
| `crates/veil-ui/src/editor.rs:87` | `Editor::new` | `pub` API，测试与无配置调用方使用 | 保留 |
| `crates/veil-ui/src/tokens.rs:38/49/180/195` | `HIGH_SHADOW_*`、`high_shadow_alpha`、`space::L` | `pub` 令牌；`view/frame.rs` 与 tokens 测试消费中 | 保留 |
| `crates/veil-ui/src/theme.rs:292/302` | `SurfaceLevel` / `Theme::surface` | `pub` API，theme 测试锁定四档映射 | 保留 |
| `crates/veil-core/src/syntax/model.rs:829` | `HTML_INLINE_WHITELIST` | `pub` 常量，供 export/paint 层交叉验证 | 保留 |
| `crates/veil-ui/src/paint_table.rs:641` | 测试辅助 | `cfg(test)` | 保留 |
| `crates/veil-ui/src/view/source_layout.rs:26` | `#![cfg_attr(not(test), allow(dead_code))]` | 非测试构建的交互/单测共用 API | 保留 |
| `crates/veil-core/tests/common/mod.rs:9`、`crates/veil-core/tests/pathological.rs:6` | 测试夹具 | `tests/` 夹具 | 保留 |

### 4.1 被模块级 `#![allow(dead_code)]` 掩盖的死代码

`crates/veil-ui/src/view/settings.rs:6` 对整个模块关闭了 `dead_code`。临时移除该项后 `cargo check -p veil-ui --all-targets` 暴露 4 项（lib 与 lib test 目标均未使用）：

| 符号 | 说明 | 处置 |
|---|---|---|
| `SettingsField::buffer` | `pub(crate)` 字段「数字输入框打字缓冲」，全仓 14 处初始化均为 `buffer: None`，无任何读取 | **删除** |
| `SettingsField::value_is_valid` | 仅被 `all_valid` 调用 | **删除** |
| `SettingsForm::all_valid` | 全仓 0 调用 | **删除** |
| `SettingsState::on_hover` | 全仓 0 调用（`app.rs` 的 `on_hover` 属模态状态，非本类型） | **删除** |

删除后该模块不再需要 `dead_code` 抑制，将一并移除以恢复 lint 可见性。

### 4.2 只被注释提到的私有函数

全仓检索「函数名仅出现在注释/文档中」的情况，除上述 `query_canvas_trace` / `query_canvas_svg`（`bootstrap.js` 注释保留 JS 侧数组，无 Rust 调用）外未发现其他项。

## 5. 疑似过时的已跟踪文档（仅列清单，不删除）

| 路径 | 最后修改 | 疑似替代 / 说明 |
|---|---|---|
| `docs/review/G0-baseline.md` | 2026-10-26 | 阶段门槛基线（573 测试时代），现行评估见 `docs/review/evaluation/evaluation.md`、`EVALUATION-FINAL-v3.0.md` |
| `docs/review/G1-gate.md` | 2026-11-03 | 同上，历史门禁记录 |
| `docs/review/G2-gate.md` | 2026-11-03 | 同上 |
| `docs/review/P0-feature-parity.md` | 2026-10-26 | 功能对标快照，已被 `docs/review/mainstream-gap-analysis.md`、`docs/review/component-inventory.md` 覆盖 |
| `docs/review/P1-commonmark-conformance.md` | 2026-10-26 | 解析一致性，最新为 `docs/review/evidence/parser-differential-2026-11-26.md` |
| `docs/review/P2-benchmark.md` | 2026-10-30 | 0.1.0 基准，最新为 `docs/review/evidence/perf/perf-gate-2026-11-26.md` |
| `docs/review/keystroke-attribution.md` | 2026-11-03 | 击键归因基线，最新为 `docs/review/evidence/perf/current-head-2026-11-26.md` |
| `docs/review/3e-manual-walkthrough.md` | 2026-11-16 | 阶段 3 人工清单，现行人工验收见 `docs/manual-test-veil.md` |
| `docs/review/component-replacement-spike.md` | 2026-11-21 | 组件替换 spike，结论已并入 `docs/review/component-inventory.md` |
| `docs/analysis-architecture-and-competitor-gap.md` | 2026-11-20 | 文内自述为 2026-11-13 快照，架构现状见 `docs/architecture-layering.md` |
| `docs/perfect-writing-surface-strategy.md` | 2026-11-24 | 文内自述「历史文档，保留原貌」，现状见 `README.md` / `docs/04-strategy-execution-plan.md` |
| `docs/table-edit-design.md` | 2026-11-03 | 设计文档，布局约束已固化于 `docs/adr/ADR-013-table-layout.md` |
| `docs/adr/ADR-003-mark-bias-affinity.md` | 2026-10-24 | ADR 属历史决策记录，**不建议**删除 |
| `docs/review/v1-2-implementation-log.md`、`v1-7-baseline-review.md` | 2026-11-09 / 11-15 | 版本留痕，**不建议**删除 |

## 6. 清理执行结果（T2 / T3）

- T2：删除 `target/`（14,920 文件 / **22.86 GB**）。仓库根与 `docs/` 无其它可删的 `*.log` / `*.tmp` / 临时导出；`docs/review/evidence/ux/*.log` 因 `docs/review` 受保护而保留。
- T2 首次全量 `cargo test --workspace`：`platform::http::tests::actual_loopback_peer_fails_closed` 在并行时命中已知环回 flake（WinHTTP 错误码 12152），单独重跑通过；其余用例全绿。
- T3：删除 6 个零引用私有符号——`Session::query_canvas_trace`、`Session::query_canvas_svg`、`SettingsField::buffer`、`SettingsField::value_is_valid`、`SettingsForm::all_valid`、`SettingsState::on_hover`，并移除 `view/settings.rs` 模块级 `dead_code` 抑制。
- T3 后 `cargo check --workspace --all-targets` **零告警**；`cargo test --workspace` **全部通过**（2,176 passed / 0 failed / 17 ignored）。
- 清理后 `target/`（测试重建的 debug 产物）：3,101 文件 / 约 **4.0 GB**，较清理前下降约 18.8 GB。

