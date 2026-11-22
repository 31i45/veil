# evaluation-subtract.md — subtract-surface-and-modularize 复核报告

> 状态：**已完成、门禁全绿、已本地提交（未推送）**。
> 日期：2026-11-22。基线：v3.0.0（`veil.exe` 14,012,928 B / `FF23C4D9…DD25D0`，`veil-ui` 1281 passed / 11 ignored）。

## 0. 结论

**`PASS`**。本轮在**不删任何用户可见功能**的前提下完成内部减法与模块化：最大源文件从 17,960 行降到 <3000 行，`veil-ui` 74 个文件中 ≥3000 行者为 **0**；配置字段 11→10；清掉 9 处多余 `#[allow(dead_code)]` 与 1 个死字段。全部门禁与自动化回归通过，无行为回归。三次本地提交，未推送 GitHub。

**诚实边界**：用户可见的功能裁剪（U-A 删缩放 / U-B 删专注模式 / U-C 固化 doc_line_height）因用户未批准而**全部保留（keep）**。本轮"减法"仅指内部死代码与隐藏配置的移除；可感知表面积（菜单项、设置项）**未减少**。文件规模与配置数下降是本轮的真实收益，功能入口裁剪待用户批准后另行落地。

## 1. 三次本地提交（未推送）

| 提交 | 内容 | 阶段 |
|---|---|---|
| `78f37c4` | 内部减法：固化 `image_cache_limit_mb`、清死代码 | Task 3 |
| `71282a8` | 拆 `app_tests.rs` 为 10 个主题模块 | Task 4 |
| `86b6d9c` | 拆 layout / interaction / paint / editor | Task 5 |

`git status -sb` = `## main...origin/main [ahead 3]`；无 push 记录。

## 2. 减法（Task 3，78f37c4）

| 项 | 前 | 后 |
|---|---:|---:|
| `Config` 字段数 | 11 | **10**（移除 `image_cache_limit_mb`） |
| `#[allow(dead_code)]` | 19 | **10**（删 9 处多余；10 处编译器证实必需，保留） |
| 死字段 | `BgResult.epoch`（只写不读） | 已删除（含写入点与测试字面量） |
| `veil-ui` 行数（本阶段） | 63,131 | 63,101（−30） |

- 迁移：旧 `config.toml` 含 `image_cache_limit_mb` 时解析静默忽略、保存时剥离，不崩溃、不报"无法识别"。已用含旧键的独立 `VEIL_HOME` 启动验证（进程存活、stderr 无 panic）。

## 3. 模块化（Task 4 + Task 5）

| 文件 | 拆前 | 拆后 | 新模块 |
|---|---:|---:|---|
| `app_tests.rs` | 17,960 | —（删除） | `app_tests/` 10 文件，最大 `mod.rs` 2576 |
| `view/layout.rs` | 6,830 | 2,547 | `layout_engine.rs` / `layout_tests.rs` / `layout_inline_tests.rs` |
| `interaction.rs` | 4,578 | 2,970 | `interaction_menu.rs` |
| `paint.rs` | 3,967 | 1,988 | `paint_table.rs` / `paint_overlay.rs` |
| `editor.rs` | 3,337 | 2,936 | `editor_helpers.rs` |

- 全部为**纯移动**：未改任何函数逻辑、字段语义、常量值、测试断言；共享项用 `pub(crate)/pub(super) use` 保持既有 `crate::` 路径。
- 最终 `crates/veil-ui/src` 74 个 `.rs` 文件，**≥3000 行者为 0**（最大 `interaction.rs` 2970 / 计行 2843）。
- seam 示例：`ctx_exec`（右键动作分发）从 4,578 行 `interaction.rs` 移到 1,621 行 `interaction_menu.rs`；改菜单动作不再穿越输入 / 几何代码。

## 4. 全门禁（本次 release 构建）

| 门禁 | 结果 |
|---|---|
| `cargo build --release -p veil` | **passed**（exit 0） |
| 体积 | **14,011,904 B** ≤ 14,680,064 B（比 v3.0.0 少 1,024 B，减法生效） |
| SHA-256 | `324D3EEC995935AD2437DD75459247A0BAEEF16541FDAEAA781AB18002669377` |
| 离线特征串 | `QuickJS` / `linkedom` / `mermaid` 均命中 |
| `cargo fmt --all -- --check` | **passed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **passed**（零警告） |
| `cargo test --workspace --release` | **passed**：`veil-ui` **1281 passed / 0 failed / 11 ignored**；workspace 合计 1,968 passed / 0 failed |

> 注：本次 release 测试用默认并行（veil-ui 187s），未用 v3.0.0 的 `--test-threads=1`（该模式单线程下 `real_event_loop` 在高负载时偶发 10s 窗口可见性时序抖动；并行模式下未触发）。pass 数与基线一致，无回归。

## 5. 自动化回归（AC-7）

| 项 | 结果 |
|---|---|
| `walkthrough.ps1 -EvidenceMode input` | **PASS**（HTML 导出断言、正常关窗 exit 0） |
| `walkthrough.ps1 -EvidenceMode startup` | **PASS**（DPI=150 首帧黑像素 0% @300/800ms） |
| `walkthrough.ps1 -EvidenceMode d1` | **PASS**（冷启动 n=10 p50=106.93ms / p95=110.32ms，idleWS=43.87 MiB） |
| `perf_20k_blocks` | **PASS**：击键 p50=2.484ms / **p95=2.827ms** / max=3.082ms（基线 ~2.6ms，+8.7% ≤ 10% 阈值） |
| `smoke_launch.ps1` | **PASS**（launch 561ms ≤ 1000ms；隔离 PASS，未触碰真实 `%APPDATA%\Veil`） |

## 6. Rubric 评分

| AC | 维度 | 分 | 理由 |
|---|---|---:|---|
| AC-9 可维护性 | 改一个动作需触碰的文件数 / 行数 | **4** | `ctx_exec` 改动足迹从"在 4,578 行大文件里找"降为"在 1,621 行专用模块里改"；模块边界可指认。未到 5 是因为 `interaction.rs` 仍有 2,970 行，菜单与输入的 trait 接口未完全抽象。 |
| AC-10 减法克制 | 是否只删低价值项、核心流零感知 | **5** | 未删任何用户可见功能；仅移除死代码与隐藏配置，核心写作流（新建/打开/输入/保存/导出）完全无感知，旧配置平滑迁移。 |

## 7. 仍未验证（NOT RUN，不写成 PASS）

以下属后续 spec 或需用户手工，本轮不涉及、状态不变：

- 真实中文 IME 候选窗 / 组合态（B2）—— NOT RUN，待 `freeze-input-fidelity` + 用户手工。
- 100/125/200% DPI 视觉（E4）、实体打印（F3）—— NOT RUN，待工艺阶段 + 用户手工。
- 真实断网抓包结论（G1/G2）—— NOT RUN，待分发阶段 + 用户签字。
- 真实 GitHub Actions runner、Linux 桌面—— NOT RUN / 不在范围。
- 用户可见功能裁剪（U-A/U-B/U-C）—— 未批准，保留 keep。

## 8. 下一步（乔布斯式）

1. **本 spec 的"减法"只完成了一半**：模块化（文件规模）达标，但**功能入口裁剪未做**。若你批准 U-A/U-B/U-C，我用独立提交做掉，可随时 `git revert`。
2. **`freeze-input-fidelity`** 的实现现在可以更便宜地推进——菜单 / 输入 / 布局已分离，改 IME 与撤销不再需要穿越 4,000 行大文件。其实现可与手工验收并行。
3. 推送与否由你决定：当前本地领先 `origin/main` 3 个提交，随时可 `git push`（本轮按要求未推送）。
