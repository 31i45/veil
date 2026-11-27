# Veil 交付事实（2026-11-27）

> 本页为人工维护的交付事实。`report-2026-11-27.md` 由语法矩阵测试生成，运行测试会覆写，请勿手工追加。

## 代码

- 未推送 GitHub、未打 tag、未修改版本号。
- 未新增任何依赖（工作区 Cargo.toml / Cargo.lock 无变化）。

## 测试

- `cargo test --workspace`：全部套件 **0 failed**。
- 分套件：`veil` lib 274（1 ignored）、`docs_policy` 6、`syntax_matrix` 4、`veil-core` lib 347（1 ignored）、`conformance` 1、`core` 86、`gfm_conformance` 1、`parser_differential` 2、`real_world_check` 1、`pathological` 0（1 ignored）、`veil-mermaid-offline` 26（2 ignored）、`veil-ui` lib 1461（12 ignored）、`boundary_guard` 7。
- 环境性偶发：`platform::http::tests::actual_loopback_peer_fails_closed` 在首次全量运行报 WinHTTP 12152，单独重跑与随后全量复跑均通过；未修改断言、未忽略用例。

## 静态检查

- `cargo clippy --workspace --all-targets -- -D warnings`：0 warnings。
- `cargo fmt --all --check`：通过。

## 发布产物

- `target/release/veil.exe` = **14,276,608 B**（门禁 14,680,064 B，余量 403,456 B）。
- SHA-256 = `F2CA58097477435DFC271E9F4B0BF0E7E8F208368DEEE44FA2A08C15BB4D581B`。

## 语法矩阵

- 163 用例 × 7 表面 = **1141 PASS / 0 FAIL / 0 NOT RUN**。

## 真机走查

- 浅色 · 所见即所得：240 / 320 / 480 / 800 宽。
- 浅色 · 源码模式（Ctrl+/）：800 宽。
- 深色 · 所见即所得（`VEIL_HOME` 隔离配置，未触碰用户配置）：800 宽。
- 截图证据在临时目录 `%TEMP%\veil-spec-walkthrough\`（不入库）。
- 专注/打字机模式的结构与分页由语法矩阵七表面覆盖；专注模式压暗对比度由单测锁定（被压暗正文 ≥3:1）。

## 设计系统变更摘要

- 浅色语法 7 个 token 全部 ≥4.5:1；深色标点 4.78:1。
- 屏幕 / HTML / PDF 标题字号同源于 `type_scale`；HTML 导出补齐 h1–h6，h5/h6 不小于正文。
- 导出 `mark` 与公式失败配色改由主题派生，深色导出不再出现亮黄块。
- alpha 阶梯单源化；专注/拖放压暗收敛到 40% 档。
- 空态改用中性色；异常四态矩阵 44 格成文，G1/G2/G3/G5 已修，G4/G6 记为延后。
- 术语表 18 条；英文标签预算 39 字符；应用内图标不建系统，仅单色语义徽标。
