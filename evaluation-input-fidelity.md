# evaluation-input-fidelity.md — 输入保真冻结复核

> 日期：2026-11-22。基线：v3.0.0 后 `veil-ui` 1281 passed / 11 ignored。本轮仅本地提交，未推送。

## 0. 结论

**`PASS`**。三个真实输入缺陷已修复：IME 上屏并入先前键入的撤销组、空 IME 提交置脏并删除选区、提交落入菜单/历史后组合态残留。新增 8 个确定性测试与 1 个 50 MB 探针；全门禁绿色，50 MB 长文打开 711.9 ms、击键 p95 2.291 ms。

## 1. 修复项

| 缺陷 | 根因 | 修复 | 回归测试 |
|---|---|---|---|
| 上屏撤销吞掉先前键入 | `commit_ime` 直接 `InsertText`，单字符提交与键入同组 | `set_preedit` 在组合开始断开撤销组 | `ime_commit_is_separate_undo_step_after_typing`、`ime_multichar_commit_is_single_undo_step` |
| 空提交置脏 / 删选区 | `InsertText("")` 返回 `Some`，选区分支执行替换 | `commit_ime("")` 直接返回空脏块 | `empty_ime_commit_is_noop_with_clean_document`、`empty_ime_commit_keeps_selection` |
| 菜单/历史组合态残留 | 提交分支提前 return 未清 preedit；preedit 只拦 History | Menu 下拒绝挂载；两条落点均清理 | `ime_preedit_cleared_when_commit_lands_in_menu_or_history` |

## 2. 测试与证据

| 项 | 结果 |
|---|---|
| `mixed_edit_sequence_is_fully_reversible` | PASS：CRLF 文档交错键入/换行/粘贴/替换/IME/移动，全撤销回初始、全重做回终态 |
| `committed_ime_text_survives_crash_recovery` | PASS：组合期不写快照；上屏后快照含文本与路径；重启逐字恢复并置脏 |
| `truncated_and_non_utf8_recovery_are_rejected` | PASS：截断 / 非 UTF-8 / 坏魔数均拒绝并正常打开 |
| 既有恢复测试 | 保留：未保存即崩溃 → 重启恢复 → 保存收口 |

## 3. 50 MB 长文（release，独立探针）

| 指标 | 实测 | 阈值 |
|---|---:|---:|
| 文档 | 53,708,890 B / 20,001 块 | ≥50 MB |
| 打开 + 首帧 | **711.9 ms** | ≤5,000 ms |
| 击键 p50 | **2.027 ms** | — |
| 击键 p95 | **2.291 ms** | ≤16 ms |
| 击键 max | 141.364 ms | 冷启动首个样本 |
| 保存 | 349.8 ms | — |
| 保存字节 | 与会话序列化逐字节一致 | 必须一致 |

未做性能优化，也未调整口径。

## 4. 全门禁

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS（零警告） |
| `cargo test --workspace --release` | PASS：**1,976 passed / 0 failed**；`veil-ui` **1289 passed / 0 failed / 12 ignored** |
| `cargo build --release -p veil` | PASS |
| `veil.exe` 体积 | **14,012,416 B** ≤ 14,680,064 B |
| SHA-256 | `64EAFA1AD6989FD0F48759B376C4DEDBD06DE27332DF05DA1FF11CEC6DC28D2F` |
| 离线特征串 | `QuickJS` / `linkedom` / `mermaid` 均命中 |
| smoke | PASS：launch 594 ms；`VEIL_HOME` 隔离 PASS，真实 APPDATA 哨兵未变 |

测试数较基线 +8，ignore +1（新增 50 MB 探针），无测试被删除。

## 5. Rubric

| AC | 维度 | 分 | 理由 |
|---|---|---:|---|
| AC-10 输入保真质量 | 事务边界是否单点、可解释 | **4** | 撤销边界收在 `Editor::set_preedit`，空提交收在 `commit_ime`，落点清理收在 `App::ime_commit`/`ime_preedit`；测试直接对应协议。未到 5 是因为组合态仍由 Editor 与 App 两层共同维护。 |

## 6. NOT RUN（不写成 PASS）

- 真实微软拼音候选窗跟随、组合下划线观感、候选框位置（B2）——需用户手工录屏。
- 真实任务管理器强杀后的恢复提示观感（C7）——代码级恢复已覆盖，UI 观感未验证。
- 其余四档 DPI、实体打印、抓包、GitHub Actions / Linux 均不在本 spec 范围。

## 7. 下一步

- `craft-four-dpi-and-distribution-trust`：四档 DPI、签名/SHA/SmartScreen/抓包/Actions。
- 本地已领先 `origin/main` 7 个提交（含上一 spec 4 个），按要求未推送。
