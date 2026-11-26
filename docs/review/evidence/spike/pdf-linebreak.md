# PDF 断行子模块 spike（Task 9 · Agent I）

> 日期：2026-11-26
> HEAD：`c8298b5`（Phase 3 完成态）；版本 `3.0.2`；工具链 rustc/cargo 1.99.1
> 范围：**test-first spike，不改生产 PDF 输出**。未触碰 UI / config / parser / export。
> 结论：**NO-GO（本阶段不替换生产断行器）**；保留 dev-dependency 与可复现对比测试。

## 1. 被测对象

| 侧 | 实现 | 位置 |
|---|---|---|
| 当前策略（基线） | 贪心装行 + 简化中文禁则（`fill_lines` / `kinsoku_break`） | `crates/veil/src/platform/pdf_layout.rs:184` / `:300` |
| 候选 | `unicode-linebreak` 0.1.5（UAX #14 断点机会，dev-dependency） | `crates/veil/Cargo.toml` `[dev-dependencies]` |
| 未采用候选 | `icu_segmenter` 2.4.0（不引入，评估见 §5） | — |

对比工具：`crates/veil/src/platform/pdf_linebreak_spike_tests.rs`
（以 `#[path]` 挂在 `pdf_layout::tests` 下，仅测试期编译）。

复现：

```powershell
cargo test -p veil pdf_linebreak_spike -- --nocapture
```

## 2. 方法

- **同一容量**：PDF 正文版心 `A4(595.28pt) − 2×20mm = 481.89pt`，正文 11pt ⇒ **43.81em**。
- **同一字宽**：两侧都用 `FakeMetrics`（ASCII 0.55em、CJK 1.0em、空格 0.28em），排除字体 advance 干扰。
- **同一语料**：沿用 Task 7 的 CJK / 闭符号 / 开符号 / 半角 / 破折号 / 中西混排 / 超长词 / 表格单元格。
- **候选装行**：`unicode_linebreak::linebreaks` 给出允许/强制断点，再用同一字宽贪心装行；无允许断点时硬断，保留强制断点（硬换行）。
- **比较口径**：去空白后的逐行内容、可见字符累计断点、行首禁则违规、行宽溢出、丢字/空行。
  行首禁则的「统一目标」取 PDF 自研表 ∪ 屏幕表独有项（`—` `～`），与 Task 7 §4.1 一致。

局限：策略层比较，不建模链接标注/图片盒/样式（`LItem`），也不使用真实字体度量；混合语料只覆盖 1 组。

## 3. 基线数据（当前策略 vs 候选）

`--nocapture` 实测摘要（容量 43.81em）：

| 语料 | 行数（当前/候选） | 断点（可见字符累计） | 不同行 | 最大行宽 em（当前/候选） | 行首禁则违规（当前/候选） |
|---|---:|---|---:|---|---|
| `cjk`（汉×120） | 3 / 3 | `[43,86]` 完全一致 | 0 | 43.00 / 43.00 | 0 / 0 |
| `cjk_punct`（甲×42+。×3） | 3 / 3 | `[43,86]` 完全一致 | 0 | 43.00 / 43.00 | 0 / 0 |
| `cjk_open`（乙×42+（×5+后） | 6 / 6 | `[42,85,128,171,214]` 一致 | 0 | 43.00 / 43.00 | 0 / 0 |
| `halfwidth`（丁×43+`...ABC`） | 2 / 2 | `[42]` 一致 | 0 | 42.00 / 42.00 | 0 / 0 |
| `em_dash`（甲×43+—+乙×10） | 2 / 2 | `[43]` 一致 | 0 | 43.00 / 43.00 | 1（第 2 行 `—`）/ 1 |
| `mixed`（中×20 + ascii + 文×20） | 2 / 2 | `[34]` vs `[48]` | **2** | 28.82 / 42.82 | 0 / 0 |
| `longword`（x×200） | 3 / 3 | `[79,158]` 一致 | 0 | 43.45 / 43.45 | 0 / 0 |
| `table_cell`（表×4000） | 94 / 94 | 每 43 字，全部一致 | 0 | 43.00 / 43.00 | 0 / 0 |

**合计不同行：2 / 共 118 行；8 组语料中 7 组断点逐行完全一致。**

`mixed` 的差异（唯一实质差异）：

```
current  = ["中×20 ascii words here ", "文×20"]        // 首行 28.82em，止于空格（词边界）
candidate= ["中×20 ascii words here 文×14", "文×6"]     // 首行 42.82em，止于 CJK 边界
```

- 当前策略保留 `last_space` 偏好，把 ASCII 短语整体留在一行，行更短但词边界完整。
- 候选取容量内最后一个 UAX 允许断点（CJK 之间），把 ASCII 短语与后文混排挤满。
- 两者都不丢字、不溢出、不违反各自禁则；属排版偏好差异，候选无质量优势。

硬换行 / 空行（候选）：

- `第一行\n\n第三行` ⇒ 3 行，中间空行保留（强制断点）。
- `x×200` @60em ⇒ 多行硬断，拼接无丢字，无行溢出。
- 生产按块切段，换行不会进入 `fill_lines`；此项只证明候选的强制断点语义完整，不改变现有契约。

## 4. 关键发现

1. **当前自研策略在这些语料上与 UAX #14 断点等价**（7/8 组逐行一致，含纯 CJK、闭/开符号、半角 `.`、破折号、超长词、4000 字表格单元格）。
2. **Task 7 的「半角 `.` 在 UAX 语境下未断言」已有答案**：本语料下 UAX #14 同样不在 `.` 前断行，候选与当前一致，不构成回归。
3. **候选不能修复已知缺口**：`—` 在 UAX 中属 B2（前后可断），当前与候选都会把 `—` 放到行首；屏幕靠 WJ 禁止。Task 7 §4.1 的「`—`/`～` 待裁决」不会因换用 `unicode-linebreak` 而消失。
4. **候选只接受 `&str`**：不携带 `LItem` 的链接开闭/图片盒/样式，无法直接替代 `fill_lines`；接入需重写行装配与链接跨行切分。

## 5. `icu_segmenter` 评估（不引入）

| 项 | 数据（crates.io，2026-11-26） |
|---|---|
| 版本 / 许可证 | 2.4.0 / **Unicode-3.0** |
| crate 压缩体积 | 4,474,652 B（≈4.47 MB） |
| 声明的依赖 | 18 条：7 个必需运行时（`icu_collections`、`icu_provider`、`potential_utf`、`smallvec`、`utf8_iter`、`zerotrie`、`zerovec`）+ 可选数据/回退/序列化（含 `icu_segmenter_data`、`icu_locale_fallback`、`databake`、`serde`） |
| 默认特性 | `compiled_data` + `auto`（`auto = lstm`） |
| 数据 crate | `icu_segmenter_data` 2.4.0：3,141,236 B（≈3.14 MB），Unicode-3.0，0 传递依赖 |
| MSRV | 1.88 |

不选理由：

- **体积**：仅数据 crate 压缩后 3.14 MB；当前 exe 对 14 MiB 门禁余量只有 **509,952 B**。即使 LTO + 死代码消除，也不具备可行性，且本阶段未做真实链接测量，不能假设能过门。
- **依赖/维护**：至少 7 个必需 ICU4X crate 加数据 crate，数据版本与 MSRV 需随 ICU4X 升级同步；默认 `auto = lstm` 引入泰/老/高棉/缅的 LSTM 模型，对 CJK PDF 无收益。
- **质量**：CJK 断点机会与 `unicode-linebreak` 同属 UAX #14，本 spike 已证明当前策略已等价；ICU 的增量价值只在词典分词（非本产品需求）。
- **许可证**：Unicode-3.0 与 MIT 项目兼容，但分发时需保留 notice 并更新 `THIRD-PARTY-LICENSES.md`；属可接受成本，不是决定性否决项。

## 6. 六道门逐项结论

| 门 | 候选 `unicode-linebreak` | 结论 |
|---|---|---|
| 质量 | 7/8 语料零差异；`mixed` 2 行差异且丢失空格/词边界偏好；无禁则违规、无溢出、无丢字；未修复 `—` 行首缺口 | **不通过**：无净收益，局部轻微退化 |
| 体积（生产 exe ≤14 MiB） | 仅 dev-dependency，不进入 release；实测 `veil.exe` **14,170,112 B**，与 Task 8 基线同尺寸；且该 crate 已由 `cosmic-text` 进入生产依赖图（`veil-ui`、`veil-mermaid-offline`），替换为零字节增量 | **通过**（但无意义） |
| 许可证 | Apache-2.0，**0 传递依赖**，与 MIT 项目兼容 | **通过** |
| 离线 | 纯 Rust 编译期表，无网络、无运行时数据下载 | **通过** |
| 无 webview | 纯算法库，无 webview/进程外依赖 | **通过** |
| 行为契约 | 只处理 `&str`，丢失 `LItem`（链接/图片/样式）与 `last_space` 偏好；替代需重写行装配并重跑 Task 7 的 52 项回归 | **不通过**：改动面大、收益为零 |

**GO/NO-GO：NO-GO。** 六道门中质量与行为契约为硬门槛，均不成立；体积/许可证/离线/无 webview 虽通过，但候选没有带来可测量的用户价值。

## 7. 处置与未来触发条件

处置：

- 不修改生产代码；`pdf_layout.rs` 仅新增 `#[cfg(test)]` 下的 spike 模块挂载。
- 保留 `unicode-linebreak` dev-dependency（该包已在 `Cargo.lock`，不新增包）与两个可复现测试，供后续候选对比复用。
- `Cargo.lock` 仅新增 `veil` 包依赖列表中的一行 `unicode-linebreak`；release 产物体积不变。

再次评估的触发条件（任一成立）：

1. 需要为泰/老/高棉/缅等无空格语言做词边界断行（`unicode-linebreak` 不足，需 ICU 词典/LSTM）。
2. 决定统一 PDF 与屏幕的单一真值层：用 UAX 断点驱动 `veil-ui::view::linebreak`，并把 Task 7 的表差分断言从「差异存在」改为「无差异」。
3. 现有 `fill_lines` 需要处理超过 2 个连续闭符号的避头，或需要 UAX 数值语境（IS/NU 前向）规则。
4. `cosmic-text` 升级导致屏幕 UAX 表变化，需要一个独立可跑的断行 oracle。

## 8. 验证记录

```text
cargo test -p veil pdf_linebreak_spike -- --nocapture
  test result: ok. 2 passed; 0 failed; 0 ignored （253 项中过滤后运行 2 项）

cargo build --release -p veil
  veil.exe = 14,170,112 B (13.514 MiB)
  SHA-256  = 302EB118D3D1045A57FA0A70895029ED7D3FE7663358FEF2B66C82271DC3980B
```

- 体积与 Task 8 基线 `14,170,112 B` 完全一致；SHA 与 Task 8 记录不同，属重建产物（PE 头时间戳等构建元数据），不做逐字节回归；门禁按尺寸判定。
- 未运行真机字体导出对比；本 spike 不改变导出路径，Task 7 的端到端证据仍有效。
