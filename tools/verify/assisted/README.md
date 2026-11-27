# Assisted 人机协同验证（Task 5）

需要人在场的三项（系统 DPI、实体打印机、真机 IME）统一为「一条命令启动 + 分步提示 + 自动截图/落盘 + 自动汇总」。
用户只承担无法自动化的物理动作；脚本负责其余取证，**未执行就写 NOT RUN，不伪造 PASS 或截图**。

- 证据目录：`docs/qa/evidence/assisted-2026-11-27/`
- 运行环境：PowerShell 7（`pwsh`）、Windows、已构建 `target/release/veil.exe`
- 语料：`docs/qa/syntax-matrix.md`（缺失时退回 `README.zh-CN.md`）

## 1. 系统 DPI

```powershell
# 只报告当前档位（不启动应用）
pwsh -NoProfile -File tools/verify/assisted/dpi.ps1

# 当前系统缩放已是目标档位时抓图（示例 150%）
pwsh -NoProfile -File tools/verify/assisted/dpi.ps1 -Dpi 150 -Capture
```

- **你要做**：Windows 设置 → 系统 → 屏幕 → 缩放，切到目标档位；重新登录（或完全重启 veil.exe）后再运行命令。
- **自动做什么**：读取 `GetDeviceCaps(LOGPIXELSX)` / `GetDpiForSystem` / `GetDpiForWindow` 与注册表 `LogPixels`；启动 veil.exe 打开语料；等窗口出现后再等 1.5 s 首帧稳定；用 `PrintWindow`（失败降级 `CopyFromScreen`）抓整窗到 `dpi-<Dpi>.png`；向 `dpi.jsonl` 追加一行 JSON。
- **判定**：请求档位与实测不一致（系统或窗口 DPI）→ `status=NOT_RUN`、**不产生 png**、退出码 2；一致才抓图并写 `status=PASS`。
- **核对标准**（人工看截图）：首帧完整无空白/半屏、标题与正文基线清晰、表格网格线为 1 物理像素、窗口最大化/还原后版心重新居中、光标落点与字形误差 ≤1 字符宽。

## 2. 实体打印机

```powershell
# 生成样张 + 打印机/driver 清单 + 核对清单，并打开应用
pwsh -NoProfile -File tools/verify/assisted/print.ps1

# 只生成样张与清单（不启动应用）
pwsh -NoProfile -File tools/verify/assisted/print.ps1 -NoLaunch
```

- **你要做**：装 A4 纸 → 在应用里 `Ctrl+Alt+P` → 选择清单中的实体打印机 → A4 → 打印 → 逐项核对出纸。
- **自动做什么**：生成 `print-sample.md`（短代码块、80 行跨页长表、仓库内既有 png 图片）；`Get-Printer` 枚举名称/驱动/端口/状态；生成 `print-checklist.md`（每页表头、断页行边界、页码、页数、图片完整性六项）。
- **判定**：出纸项自动无法确认，一律 `PENDING`；没有实体打印机时整节 `SKIP` 并写明原因；六项全部人工 PASS 才算达标。驱动字体度量差异按 `docs/review/evidence/layout-consistency-2026-11-26.md` 的已接受环境差异处理。

## 3. 真机中文 IME

```powershell
# 交互式六步走查（需要真人打字）
pwsh -NoProfile -File tools/verify/assisted/ime.ps1

# 不执行，仅登记 NOT RUN（CI/无人值守）
pwsh -NoProfile -File tools/verify/assisted/ime.ps1 -Preflight
```

- **你要做**：把输入法切到「微软拼音」（再跑一遍「微软五笔」）；按提示完成六步：段落中部 `nihao` 的 preedit 跟随 → 空格上屏「你好」→ `zhongguo` 选第 2 候选 → 行首/行尾/表格单元格各重复一次 → 输入中 `Esc` 取消 → 失焦再回焦。每一步做完回车。
- **自动做什么**：启动 veil.exe 并置前；逐步打印操作要点；每步回车后自动截图 `ime-step-<n>.png`；最后询问六步是否无丢字/重字/错位，生成 `ime-result.md`（六步 / 用户观察 / 截图路径 / 判定）。
- **判定**：六步均无丢字、重字、错位为 PASS；任一失败记录输入法版本、文档位置与截图；未执行记 `NOT RUN`。

## 失败如何记录

- 启动失败、窗口超时、截图全黑：脚本保留 `.blank.png` 诊断图，`dpi.jsonl` 写 `status=FAIL` + `reason`，退出码 1。
- 缺少 `veil.exe`：提示 `cargo build --release -p veil`，不生成截图。
- 档位/DPI 不一致、缺少实体打印机、未执行 IME：分别写 `NOT_RUN` / `SKIP` / `NOT RUN`，绝不改写成 PASS。
- IME 脚本检测到标准输入被重定向（自动化环境）时拒绝启动应用并写 `NOT RUN`，避免生成「用户没有真正操作」的假截图。
- 每轮执行后检查 `dpi.jsonl`、`print-checklist.md`、`ime-result.md` 的状态列与退出码，并在
  `docs/review/deferred-and-manual-register-2026-11-27.md` §1.1–§1.3 更新状态。
