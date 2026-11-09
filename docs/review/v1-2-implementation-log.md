# v1.2「手感与表达」实施记录

> 对应 spec：`.trae/specs/close-editor-gap-v1-2/`。所有数字均为本机（Windows）实测，命令与时点如下。

## 0. 基线（Task 0.1，2026-11-09）

| 指标 | 命令 | 结果 |
|---|---|---|
| 测试 | `cargo test --workspace` | 1078 passed / 0 failed / 13 ignored |
| 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| release 体积 | `(Get-Item target\release\veil.exe).Length` | 6,026,240 B（门禁 6,291,456 B，余量 265,216 B） |
| keystroke p95 | `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path` | 6.677 ms @5005 块 / 12.488 ms @10500 块（p50 6.298 / 11.806） |
| 冷启动 | `cargo run --release -p veil --example bench_typing` | 520.87 ms / 533.85 ms |

## 1. 体积 spike：为什么不做正则查找（Task 0.4）

临时引入 `regex`（`default-features = false`，特性 `std` / `unicode-perl` / `unicode-case`）后 release exe = **7,224,832 B**，净增 **1,198,592 B**。根因：`lapce-xi-rope` 硬依赖 `regex` 默认特性，feature 并集无法削减。结论：查找保持「子串 + 可选大小写敏感」，表达力诉求交给外部工具（已回退工作树）。

## 2. Wave 1（手感）

| 任务 | 净体积 | 说明 |
|---|---|---|
| Task 1 键位扩展 | +13,312 B（累计） | `Alt+↑/↓` 行移动（单 undo）、`PageUp/PageDown`（复用 90% 视口翻页 + `goal_x`）、`Ctrl+Backspace/Delete` 按词删除（复用 `word_left/right`，不跨行） |
| Task 2 查找替换 | 计入上行 | 「区分大小写」开关（默认关，会话内保持）、`Enter` 替换并前进 / `Ctrl+Enter` 全部替换 |
| Task 4 光标焦点 | 计入上行 | 聚焦 2 px 实心、失焦 1 逻辑 px 空心框；零定时器（新增行 grep 仅 1 处 `request_redraw`，且仅在焦点状态变化时） |

Wave 1 后：release exe **6,039,552 B**（余量 251,904 B）；`cargo test --workspace` 1104 passed / 0 failed / 13 ignored；clippy 0 警告。

## 3. Wave 2（排版与信任）

| 任务 | 净体积 | 实测结论 |
|---|---|---|
| Task 5 排版属性 | 计入 Wave 2 合计 | `READABLE_COLS=40` 全角当量、`WIDE_WINDOW=1600`（≤1600 px 不启用上限，保窄窗口与既有坐标断言）；`content_x()` 唯一居中源（chrome 不动）。H4/H5/H6 标记区填充 **2/4/6 个空格**做层级缩进（不新增行级缩进原语）；`gap_after` 标题 **16 px** / 正文 **10 px**，y 推进与 `total_h` 同源。未新增任何配置键 |
| Task 6 对比度 | 计入 Wave 2 合计 | 语法注释 明 **5.04:1** / 暗 **4.98:1**（目标 ≥4.5）；grid/装饰线 明 **3.34:1** / 暗 **3.55:1**（目标 ≥3.0，表头底 ≥3.05）；`syntax_and_grid_meet_contrast_targets` 同时约束明暗两主题，防一边达标一边不达标 |
| Task 7 信任显性化 | 计入 Wave 2 合计 | `Clock` 端口注入（产品层手写 `GetLocalTime`，零新依赖），状态栏「已保存 · HH:MM」/「未保存」；保存失败 → 可见提示 + 保持 dirty；CLI 打开失败 → stderr + `MessageBoxW` + `exit(1)`（纯函数 `open_failure_text` 可测）；新增 9 条测试 |

Wave 2 三任务均为纯排版/颜色/接线改动，未触碰输入路由与排版换行流。

## 4. Wave 3（卫生与验证）

| 任务 | 净体积 | 实测结论 |
|---|---|---|
| Task 8 代码卫生 | **−2,048 B** | `veil-egui` 旧名 11 → 0；`Rect` 定义 4 → 1（`geometry.rs`）；`fn wide` 4 → 1（`platform/mod.rs`，附 4 条边界测试）；导出菜单补省略号 |
| Task 9 测试与走查 | 计入合计 | `platform`/`wic` 新增 9 条测试；真机走查 12 张截图（`docs/review/evidence/v1-2-walkthrough/`）；**走查发现并修复「Alt+↓ 行移动后画面滞后」**：行移动保持块 id 与行区间不变，而布局键不含内容指纹，需显式递增 `session_rev`；补回归用例 `line_move_refreshes_layout_without_further_edit` |
| Task 10 台账 | — | README / roadmap / INSTANCE 统一：体积基线 6,026,240 B、门禁 6 MiB、测试 1131 → 1132、冷启动双指标、一致性带维度后缀 |

## 5. 终验（Task 11，2026-11-09）

| 指标 | 结果 | 门禁 | 判定 |
|---|---|---|---|
| `cargo test --workspace` | 1132 passed / 0 failed / 13 ignored | 全绿 | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 | 0 警告 | 通过 |
| release 体积 | **6,043,136 B**（较基线 +16,896 B） | ≤6,291,456 B（余量 248,320 B） | 通过 |
| keystroke p95 | 6.993 ms @5005 / 11.829 ms @10500 | 回退 ≤5%（基线 6.677 / 12.488） | +4.7% / −5.3%，通过 |
| 冷启动 | 564.29 ms / 576.97 ms | 回退 ≤20%（基线 520.87 / 533.85） | +8.3% / +8.1%，通过 |

## 6. 本轮再次确认的「不做项」

以下八条全部**不做**，且理由与竞品是否存在无关：

| # | 不做项 | 否决依据 |
|---|---|---|
| 1 | 正则查找 | 体积实测净增 1,198,592 B（§1），且违反「零新增依赖」 |
| 2 | 自动配对括号/引号 | 需在每个输入路径插入上下文判定，收益是打字便利而非正确性；`Ctrl+K` 链接插入已覆盖主要诉求 |
| 3 | 全词匹配 | 与「区分大小写」同为查找开关，第二个开关的边际收益低于界面与状态复杂度 |
| 4 | `Ctrl+Shift+K` 删除行 | `Alt+↑/↓` 已让行级操作可达，再加一个同义键位是纯冗余 |
| 5 | ≤120 ms 状态过渡动画 | 与零定时器/idle 唤醒冲突；状态切换一次性 `request_redraw` 已即时可感 |
| 6 | 光标闪烁定时器 | 用「聚焦实心 / 失焦空心」表达焦点，零定时器、零空闲唤醒 |
| 7 | 排版三项的 `config.toml` 覆盖键 | 新增配置键与「0 个新键」冲突，属为假想需求付代价 |
| 8 | 列表悬挂缩进 | cosmic-text 无行级缩进原语；改写换行流会破坏 doc↔layout 偏移的致密映射 |

明确不做的整块能力（拼写检查、协作、插件、移动端/PWA、网络/遥测/自动更新、Mermaid 全量、富文本模型）维持原裁决；付费能力只允许「外部缝」（pandoc 子进程 + 错误可见）。
