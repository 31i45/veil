# Veil GitHub Release 规范化 - Implementation Plan

## Task 1: 改写 README.md 为英文主 + Typedown 背景故事 + MIT License
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - 全文翻译现有中文 README 为英文
  - **新增背景故事段**（放在 "Veil" 标题后、架构前）：Typedown 是"my favorite Markdown editor on Windows, bar none"但启动几十秒到数分钟 → "So I built my own"
  - **架构表翻译为英文**，保留 Crate 名和职责
  - **功能列表翻译为英文**（11 点）
  - **非目标列表翻译为英文**
  - **数字红线表翻译为英文**（保留所有数值实测）
  - **许可段**：把"未定"改为"MIT License · see LICENSE"
  - 添加顶部 `[中文](README.zh-CN.md)` 相对链接
  - 相对路径（`icons/veil.ico`, `docs/adr/ADR-005.md` 等）保持不变
- **Acceptance Criteria Addressed**: AC-1
- **Test Requirements**:
  - `rule` TR-1.1: Select-String README.md "Typedown" 返回 True
  - `rule` TR-1.2: Select-String README.md "tens of seconds" 返回 True
  - `rule` TR-1.3: Select-String README.md "MIT License" 返回 True
  - `rule` TR-1.4: Select-String README.md "README.zh-CN.md" 返回 True（中文链接）
- **Notes**: 保持"单机单用户 · 纯离线 · 永不协作"设计原则为英文（"single-user single-document · purely offline · no collaboration ever"）

## Task 2: 创建 README.zh-CN.md 中文备份
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None（与 Task 1 独立）
- **Description**:
  - 基于现有中文 README（Task 1 改写前的状态）拷贝为 README.zh-CN.md
  - 顶部加 `[English](README.md)` 相对链接
  - Typedown 故事段翻译为中文（放最前）："Veil 的起源：Typedown 是我最喜欢的 Windows Markdown 编辑器，没有之一，但启动需要几十秒到数分钟，让人崩溃。所以，我决定自己复刻。"
  - 许可段保持"MIT License"（与 LICENSE 一致）
- **Acceptance Criteria Addressed**: AC-1
- **Test Requirements**:
  - `rule` TR-2.1: Test-Path README.zh-CN.md 返回 True
  - `rule` TR-2.2: Select-String README.zh-CN.md "Typedown" 返回 True
  - `rule` TR-2.3: Select-String README.zh-CN.md "README.md" 返回 True（英文链接）

## Task 3: 创建 .github/workflows/release.yml
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - `name: Release`
  - `on: push.tags: [ 'v*' ]`
  - `permissions: contents: write`
  - `jobs.release: runs-on: windows-latest`
  - Steps：
    1. uses: actions/checkout@v4
    2. uses: dtolnay/rust-toolchain@stable（with: components: clippy）
    3. uses: Swatinem/rust-cache@v2
    4. run: `cargo build --release -p veil`
    5. run: `$exe = Get-Item target\release\veil.exe; Write-Host "Veil Release: $([math]::Round($exe.Length/1MB, 2)) MiB"`
    6. 归档 zip：
       ```powershell
       New-Item -ItemType Directory -Force veil-artifact | Out-Null
       Copy-Item target\release\veil.exe veil-artifact\
       Copy-Item target\release\veil.pdb veil-artifact\ -ErrorAction SilentlyContinue
       Copy-Item README.md, LICENSE, CHANGELOG.md veil-artifact\
       Copy-Item icons\veil.ico veil-artifact\
       Compress-Archive -Path veil-artifact\* -DestinationPath "veil-windows-x86_64.zip" -Force
       ```
    7. uses: softprops/action-gh-release@v2（with: files: veil-windows-x86_64.zip）
- **Acceptance Criteria Addressed**: AC-2
- **Test Requirements**:
  - `rule` TR-3.1: release.yml 存在且 `on.push.tags` 包含 `v*`
  - `rule` TR-3.2: release.yml 包含 `windows-latest` runner
  - `rule` TR-3.3: release.yml 包含 `softprops/action-gh-release` 步骤
  - `rule` TR-3.4: release.yml permissions 包含 `contents: write`
- **Notes**: 不做 cargo clippy 或 cargo test（tag 触发 release 假设已通过 gate CI）

## Task 4: 重写 .gitignore 为完整 5 分组
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - 替换当前 8 行 `.gitignore` 为带注释分组的完整版本，分组：
    1. IDE/编辑器（.vscode, .idea, *.code-workspace）
    2. OS 临时文件（.DS_Store, Thumbs.db, Desktop.ini）
    3. Rust/Cargo（target/, **/*.rs.bk）
    4. Trae/AI 中间态（.trae/skills/, .trae/documents/, 7 个历史 specs）
    5. Skills 方法论（.skills/）
    6. 构建产物（release-bundle/, veil-artifact/, veil-*.zip）
    7. 基准产物（bench/results/, pathological-10mb.md）
    8. 真机证据（docs/review/evidence/**/*.png, *.jpg）
  - **显式保留**：`.trae/specs/release-and-roadmap/`（用 `!` 反忽略）
  - **显式保留**：`docs/adr/`（无需反忽略，不写在 gitignore 里即保留）
- **Acceptance Criteria Addressed**: AC-3, AC-4
- **Test Requirements**:
  - `rule` TR-4.1: `git status --porcelain` 输出不包含 `.trae/skills/`, `.trae/documents/`, `.trae/specs/drive-v1-excellence/` 等历史 specs
  - `rule` TR-4.2: `git ls-files` 输出包含 `.trae/specs/release-and-roadmap/spec.md`（保留被跟踪）
  - `rule` TR-4.3: `.gitignore` 含分组注释分隔线
  - `rubric` TR-4.4: 覆盖度评分；1=只 target, 3=覆盖 3 类, 5=5 类全覆盖 + 明确保留 release-and-roadmap；threshold >= 4

## Task 5: 本地模拟 release workflow（build + zip）
- **Status**: `pending`
- **Priority**: medium
- **Depends On**: Task 3
- **Description**:
  - 在本地跑一遍 release workflow 的核心步骤验证逻辑正确
  - 执行：cargo build --release -p veil → 构建 veil-artifact/ → zip
  - 验证 zip 包含 6 项（exe + pdb + README + LICENSE + CHANGELOG + ico）
  - 验证 exe 大小 4–8 MiB
- **Acceptance Criteria Addressed**: AC-5
- **Test Requirements**:
  - `rule` TR-5.1: zip 产物包含 6 项
  - `rule` TR-5.2: exe 大小 4–8 MiB

## Task 6: 最终验证 + commit
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 1–5 全部完成
- **Description**:
  - 重跑 cargo test --workspace + clippy 确认 gitignore 没排除测试文件
  - git status 确认只有：源码 + docs + CHANGELOG + LICENSE + README + README.zh-CN + icons + tools + .github + .trae/specs/release-and-roadmap + 其他必要文件
  - 一个 commit 提交
- **Acceptance Criteria Addressed**: AC-4
- **Test Requirements**:
  - `rule` TR-6.1: cargo test 全绿 + clippy 零警告
  - `rule` TR-6.2: git status 输出干净（或只有本次修改的文件）

## 任务依赖图

```
Task 1 (README 英文) ──┐
                        ├── Task 6 (验证 + commit)
Task 2 (README 中文) ──┘
Task 3 (release.yml) ──── Task 5 (模拟 build) ── Task 6
Task 4 (.gitignore) ────────────────────────── Task 6
```

Task 1/2/3/4 可并行；Task 5 依赖 Task 3；Task 6 等全部完成。
