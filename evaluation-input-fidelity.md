# evaluation-input-fidelity.md — 输入保真冻结复核

> 日期：2026-11-22。基线：v3.0.0 后 `veil-ui` 1281 passed / 11 ignored。本轮仅本地提交，未推送。

## 0. 结论

**`PASS`**（经 Review R1 整改后）。六个真实输入缺陷已修复：IME 上屏并入先前键入的撤销组、空 IME 提交置脏并删除选区、菜单/历史组合态残留、设置/模态/图片预览下 IME 上屏写入正文、恢复文件内容截断无法识别。新增 14 个确定性测试与 1 个 50 MB 探针；全门禁绿色，50 MB 长文打开 721.7 ms、击键 p95 2.406 ms。

## 1. 修复项

| 缺陷 | 根因 | 修复 | 回归测试 |
|---|---|---|---|
| 上屏撤销吞掉先前键入 | `commit_ime` 直接 `InsertText`，单字符提交与键入同组 | `set_preedit` 在组合开始断开撤销组 | `ime_commit_is_separate_undo_step_after_typing`、`ime_multichar_commit_is_single_undo_step` |
| 空提交置脏 / 删选区 | `InsertText("")` 返回 `Some`，选区分支执行替换 | `commit_ime("")` 直接返回空脏块 | `empty_ime_commit_is_noop_with_clean_document`、`empty_ime_commit_keeps_selection`、`empty_ime_commit_does_not_touch_recovery_slot` |
| 菜单/历史组合态残留 | 提交分支提前 return 未清 preedit；preedit 只拦 History | Menu 下拒绝挂载；两条落点均清理 | `ime_preedit_cleared_when_commit_lands_in_menu_or_history` |
| 设置/模态/图片预览下 IME 写入正文 | IME 提交只识别 `ui_state.modal`；设置与预览不在 `mode()` 分流 | 统一早退清理；设置/模态复用键盘路由；设置打开时禁用系统 IME | `ime_preedit_cleared_when_modal_opens`、`settings_excludes_ime_text_from_document` |
| 恢复内容截断被当成短文本 | 恢复格式只有头部长度，没有内容长度/校验 | 恢复格式升级到 `VEIL-RECOVERY-2`：内容长度 + FNV-1a 校验；旧 v1 仅兼容读取 | `rejects_content_truncated_at_char_boundary`、`rejects_content_checksum_mismatch`、`legacy_v1_files_still_open` |

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
| 打开 + 首帧 | **721.7 ms** | ≤5,000 ms |
| 击键 p50 | **2.159 ms** | — |
| 击键 p95 | **2.406 ms** | ≤16 ms |
| 击键 max | 143.666 ms | 冷启动首个样本 |
| 保存 | 335.5 ms | — |
| 保存字节 | 与会话序列化逐字节一致 | 必须一致 |

探针交替采样 ASCII 键入与 IME 中文上屏（各 50 次），覆盖两条真实输入路径。未做性能优化，也未调整口径。

## 4. 全门禁

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS（零警告） |
| `cargo test --workspace --release` | PASS：**1,982 passed / 0 failed**；`veil-ui` **1295 passed / 0 failed / 12 ignored** |
| `cargo build --release -p veil` | PASS |
| `veil.exe` 体积 | **14,014,976 B** ≤ 14,680,064 B |
| SHA-256 | `476E5277E6ECC19E80AAECE1A4686933FAF65A120AEF1F76DBAB48603C0FB5D0` |
| 离线特征串 | `QuickJS` / `linkedom` / `mermaid` 均命中 |
| smoke | PASS：launch 546 ms；`VEIL_HOME` 隔离 PASS；**隔离树**内 config/session/recovery/APPDATA 哨兵未变 |

测试数较基线 +14，ignore +1（新增 50 MB 探针），无测试被删除。smoke 的哨兵位于临时隔离树；脚本明确不枚举真实 APPDATA（`read_audit=not-covered`），因此本报告只声明“隔离树未变”，不声明“真实 APPDATA 未被读取”。

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
- 本地已领先 `origin/main` 9 个提交（含上一 spec 4 个），按要求未推送。
