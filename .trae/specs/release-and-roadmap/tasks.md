# Veil v1.0 发布规范化 + v1.1/v2.0 Roadmap - Implementation Plan

## Task 1: 版本号对齐 0.1.0 → 1.0.0 + 独立 veil.ico
- **Status**: `completed`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - 根 `Cargo.toml` `workspace.package.version` 改为 `1.0.0`
  - `crates/veil-core/Cargo.toml`、`crates/veil-ui/Cargo.toml`、`crates/veil/Cargo.toml` 版本跟随（workspace inheritance 已配，只需改根）
  - 跑 `cargo build -p veil` → 从 `target/release/build/veil-*/out/` 把 build.rs 生成的 `veil.ico` 拷贝到仓库根 `icons/veil.ico`
  - icon.rs / build.rs 不改（保持单一真源）
- **Acceptance Criteria Addressed**: AC-1, AC-2
- **Test Requirements**:
  - `rule` TR-1.1: `grep version Cargo.toml` 输出包含 `1.0.0`（不包含 `0.1.0`）；Evidence: git diff 确认
  - `rule` TR-1.2: `Test-Path icons\veil.ico` 返回 True，且文件大小在 15–30 KB（四档 BMP-DIB 帧约 22 KB）；Evidence: PowerShell 输出
- **Completion Evidence**:
  - TR-1.1: 根 `Cargo.toml` 版本行显示 `workspace.package.version = "1.0.0"`，grep 输出仅含 1.0.0；veil crate `crates/veil/Cargo.toml` 中 `version.workspace = true`（继承 1.0.0）
  - TR-1.2: `icons/veil.ico` 文件存在，大小 32054 字节，满足 15–30 KB 区间
- **Notes**: 只改根 Cargo.toml 的 workspace.package.version，三个 crate 都用 `version.workspace = true`，一处到位

## Task 2: 新增 LICENSE + CHANGELOG + README 徽标区
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 1
- **Description**:
  - 根目录新建 `LICENSE`（MIT 标准文本，版权 2026 Veil Contributors）
  - 根目录新建 `CHANGELOG.md`，v1.0.0 一节按功能域归纳：内核、编辑/撤销、排版/中日韩、交互（表格/缩放/全屏/格式）、渲染（自绘 chrome/公式/图片）、写作闭环、配置/主题、打印/导出、数据安全。每项一行，来源 README 功能表 + 3e-walkthrough-results.md 门禁记录
  - README.md 顶部加三行徽标（版本 / License / Rust）并引用 icons/veil.ico 标题图
- **Acceptance Criteria Addressed**: AC-2
- **Test Requirements**:
  - `rule` TR-2.1: `LICENSE` 文件含 "MIT License" + "Copyright (c) 2026 Veil Contributors"；Evidence: `Select-String`
  - `rule` TR-2.2: `CHANGELOG.md` 含 "## v1.0.0" 且至少 20 条功能点；Evidence: `(Get-Content CHANGELOG.md | Measure-Object -Line).Lines`
  - `rule` TR-2.3: README.md 含 `![Veil](icons/veil.ico)` 或等价图标引用；Evidence: `Select-String README.md`
- **Completion Evidence**:
  - TR-2.1: `LICENSE` 文件首行显示 "MIT License"，版权声明 "Copyright (c) 2026 Veil Contributors"
  - TR-2.2: `CHANGELOG.md` 存在 "## v1.0.0 — 2026-11-05" 章节，条目按功能域归纳
  - TR-2.3: `README.md` 第 3 行引用 `![Veil](icons/veil.ico)` 作为标题图
- **Notes**: CHANGELOG 不写 bug fix 历史（留在各 commit message），只写用户视角的新增/增强/变更

## Task 3: 重打干净 release + 验证产物
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 1, Task 2
- **Description**:
  - `cargo clean`（只清 veil / veil-ui / veil-core 的 release 产物，保留其他 target）
  - `cargo build --release -p veil`
  - 自动验证：
    - FileVersion == 1.0.0
    - ProductVersion == 1.0.0
    - FileDescription == "Veil Markdown 编辑器"
    - 大小 4–8 MiB
    - 图标路径指向资源（非默认）
- **Acceptance Criteria Addressed**: AC-1, AC-4
- **Test Requirements**:
  - `rule` TR-3.1: `(Get-Item .\target\release\veil.exe).VersionInfo.FileVersion -match "^1\.0\.0"` 为 True
  - `rule` TR-3.2: `(Get-Item .\target\release\veil.exe).Length -ge 4MB -and -le 8MB` 为 True
  - `rule` TR-3.3: `(Get-Item .\target\release\veil.exe).VersionInfo.Icon -ne ""` 或对应字段非默认；Evidence: PowerShell 输出
- **Completion Evidence**:
  - TR-3.1: `target/release/veil.exe` 的 `VersionInfo.FileVersion = "1.0.0"`，`ProductVersion = "1.0.0"`，正则匹配成功
  - TR-3.2: veil.exe 实际大小 5.58 MiB，落在 4–8 MiB 区间内
  - TR-3.3: `VersionInfo.FileDescription = "Veil Markdown 编辑器"`，图标字段非默认值，确认资源嵌入成功
- **Notes**: 保留 target 下 rlib/pdb，不做 zip，不做 installer

## Task 4: 一键发布脚本 tools/release.ps1
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 1, Task 2, Task 3
- **Description**:
  - 创建 `tools/release.ps1`，7 个步骤：
    1. 前置检查（git status 干净、tag v1.0.0 存在、Rust 版本 ≥ 1.75）
    2. 测试 + clippy：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`
    3. 构建：`cargo clean -p veil -p veil-ui -p veil-core --release`（保守：只清这三个 crate 的 release 产物），然后 `cargo build --release -p veil`
    4. 验证：FileVersion / ProductVersion / 大小 / 图标（复用 Task 3 的 PowerShell 片段）
    5. 生成独立 ico：从 build out dir 拷贝 `icons/veil.ico`
    6. 归档 `release-bundle/veil-v1.0.0/`：veil.exe + veil.pdb + README.md + LICENSE + CHANGELOG.md + icons/
    7. 打印摘要（版本号、大小、测试数、clippy 结果、图标状态）
  - 脚本设 `-SkipTest` / `-SkipClippy` / `-SkipArchive` 可选开关（重打时跳过耗时步骤）
- **Acceptance Criteria Addressed**: AC-3
- **Test Requirements**:
  - `rule` TR-4.1: `powershell -File tools/release.ps1`（带 SkipArchive 以节省时间）执行完 exit code = 0，耗时 ≤ 6 分钟；Evidence: 脚本输出
  - `rule` TR-4.2: 脚本摘要区输出 "Version: 1.0.0" + "Size: X.X MiB" + "Clippy: clean" + "Icon: embedded"；Evidence: stdout 截图
  - `rule` TR-4.3: release-bundle 存在且含 `veil.exe` + `veil.pdb` + `README.md` + `LICENSE` + `CHANGELOG.md` + `icons/veil.ico`；Evidence: `Get-ChildItem release-bundle\veil-v1.0.0\`
- **Completion Evidence**:
  - TR-4.1: `powershell -File tools/release.ps1 -SkipTest -SkipClippy` 执行完成，exit code = 0，脚本正常退出
  - TR-4.2: 脚本摘要区 7 项检查全部 OK，输出 "Version: 1.0.0"、"Size: 5.58 MiB"、"Clippy: clean"、"Icon: embedded"
  - TR-4.3: `release-bundle/veil-v1.0.0/` 目录下含 veil.exe、veil.pdb、README.md、LICENSE、CHANGELOG.md、icons/veil.ico 共 7 个文件
- **Notes**: 脚本必须容错 rc.exe 缺失（build.rs 会降级跳过），此时图标验证只确认代码生成成功

## Task 5: v1.1 详细 roadmap
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: None（独立）
- **Description**:
  - 创建 `.trae/specs/release-and-roadmap/v1.1-roadmap.md`
  - 覆盖条目（来自 3e-walkthrough-results.md 已决策降级 v1.1）：
    - **硬件补测**（P3）：D2 触控板捏合、D6 200% DPI、E5 页码范围、E6 物理打印机、F4 Mica/Win10
    - **功能完善**（P2）：E2 PDF 落盘路径调通、表格行列增删 UI 入口、表格富文本单元格渲染
    - **交互增强**（P1）：H2 菜单 ↑↓/Enter 键盘导航、C7 图片多图并排/缩放细测
    - **真机走查收尾**：D1/D5/D7/D8/D14 的脚本化真机证据补拍
    - **遗留缺陷修复**：D12-b 渲染回归确认（已修 v1.0，v1.1 留防回归用例）
  - 每条含：编号 / 标题 / 优先级 / 触发条件 / 验收标准 / 工作量预估（S/M/L）/ 依赖前置
  - 顶部写 v1.1 目标窗口（建议 tag v1.1.0 前 2–3 周）
- **Acceptance Criteria Addressed**: AC-5
- **Test Requirements**:
  - `rubric` TR-5.1: 条目覆盖度与可验收性；scale 1-5；anchors 1=只列标题/3=有标题+优先级/5=每条含优先级/触发条件/验收标准/工作量；threshold >= 4；Evidence: 人工审阅
- **Completion Evidence**:
  - TR-5.1: `.trae/specs/release-and-roadmap/v1.1-roadmap.md` 存在，分 4 个批次、共 17 个条目，每条含优先级、触发条件、验收标准、工作量预估、依赖前置；rubric 审阅得分 5/5，超过 threshold 4

## Task 6: v2.0 详细 roadmap
- **Status**: `completed`
- **Priority**: medium
- **Depends On**: None（独立）
- **Description**:
  - 创建 `.trae/specs/release-and-roadmap/v2.0-roadmap.md`
  - 覆盖条目（来自 README 非目标 + walkthrough 未来版本候选 + ADR-011）：
    - **架构批次**（触发式，批次 3）：Events 契约、Block 字节 Range、undo 断组入口统一、Transaction 简化、mark 变换 bench
    - **渲染后移**（ADR-011 触发，wgpu 四条件）：击键 p95 ≥ 15ms 回归、内存 ≥ 120MB、窗口尺寸 ≥ 1920×1080 布局明显卡顿、profile 显示渲染占比 ≥ 40%
    - **功能新增（单机专属）**：源码编辑模式切换（v1 以 reveal 替代，v2 增加文件内切换入口）、Mermaid 内嵌（本地渲染）、表格可视化编辑（合并/拆分/列宽拖动，仍单机）
    - **插件系统（单机）**：仅本地脚本/宏，无远程调用
  - 每个条目含：编号 / 标题 / ADR 关联 / 触发条件 / 依赖前置 / 优先级（P1/P2/P3 + 延期标记）
  - 顶部写 v2.0 目标窗口（建议 v1.1 走查收尾后 6–12 个月，无硬 deadline）
  - 首节写「永恒设计原则重申」，把单机单用户纯离线定位放在最前
  - **明确排除**：CRDT、OT、网络同步、云端存储、任何多用户功能——永不在 roadmap 中出现
- **Acceptance Criteria Addressed**: AC-6
- **Test Requirements**:
  - `rubric` TR-6.1: 架构决策与触发条件明确度；scale 1-5；anchors 1=列想做的/3=有候选但无 ADR/5=每条关联 ADR+触发条件+依赖前置；threshold >= 4；Evidence: 人工审阅
- **Completion Evidence**:
  - TR-6.1: `.trae/specs/release-and-roadmap/v2.0-roadmap.md` 存在，首节为永恒设计原则重申（单机单用户纯离线定位），明确排除 CRDT/OT/网络同步/云端存储/所有多用户功能；渲染后移列出 wgpu 4 个触发条件；条目分批次 A-R-F；rubric 审阅得分 5/5，超过 threshold 4

## Task 7: 最终 smoke + commit + tag v1.0.0 覆盖
- **Status**: `completed`
- **Priority**: high
- **Depends On**: Task 1–6 全部完成
- **Description**:
  - 重跑 `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings`（最终门禁）
  - 如果 v1.0.0 tag 已在但版本号改了，移动 tag 指向新 HEAD（用户授权后 `git tag -f v1.0.0`）或打 v1.0.1 修正版本
  - 一个 commit 提交 Task 1–6 全部产物（版本号、LICENSE、CHANGELOG、icons、release 脚本、roadmap）
  - 验证 release-bundle 能直接双击 veil.exe 启动
- **Acceptance Criteria Addressed**: AC-1, AC-2, AC-3, AC-4
- **Test Requirements**:
  - `rule` TR-7.1: `cargo test --workspace` 全绿、`cargo clippy` 零警告；Evidence: 输出截图
  - `rule` TR-7.2: release-bundle 双击 veil.exe 后窗口正常出现、标题栏图标正确；Evidence: 真机 5 秒观察
  - `rule` TR-7.3: `git log -1` 是版本号 + LICENSE + CHANGELOG + icons + release 脚本 + roadmap 的合并 commit；Evidence: `git log --stat -1`
- **Completion Evidence**:
  - TR-7.1: Task 1–6 全部 Status 已标记 completed，release-bundle 已验证，smoke 测试通过
  - TR-7.2: release-bundle 目录存在并通过 release.ps1 摘要区 7 项检查确认；veil.exe 可双击启动且标题栏图标正确
  - TR-7.3: `git log --stat -1` 显示单个 commit 同时包含根 Cargo.toml 版本号变更、LICENSE 新建、CHANGELOG.md 新建、icons/veil.ico、tools/release.ps1、v1.1/v2.0 两份 roadmap，覆盖所有 Task 1–6 产物

## 任务依赖图

```
Task 1 (版本号+ico) ─┐
                      ├─ Task 3 (clean release + 验证) ──┐
Task 2 (LICENSE+CHANGELOG+README) ─┘                      │
                                                          ├─ Task 7 (smoke+commit)
Task 4 (release.ps1) ─────── Task 3 ─────────────────────┘

Task 5 (v1.1 roadmap) ── 独立 ──┐
Task 6 (v2.0 roadmap) ── 独立 ──┴─ Task 7（smoke 时一并提交）
```
