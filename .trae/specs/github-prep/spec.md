# Veil GitHub Release 规范化 - Spec

## Overview
- **Summary**: 将 Veil 仓库规范到可安全推 GitHub 的清爽状态：README 双文（英文主 + Typedown 背景故事）、CI/Release workflow 完整、仓库结构干净（中间态排除、必要文档保留）。
- **Purpose**: v1.0.0 tag 已在本地打好，用户计划推 GitHub。当前三个阻塞项：(1) README 纯中文无 Typedown 背景且"许可未定"与已建 LICENSE 矛盾；(2) 只有 gate CI，无 tag 触发的 release workflow（自动打 release + 上传 artifact）；(3) .gitignore 极简不覆盖 `.trae/` 中间产物，推上去会带上所有规划中间态和真机截图。
- **Target Users**: GitHub 访客（开发者/最终用户）、CI Runner、未来维护者。

## Goals
1. README.zh-CN.md（英文主）+ README.zh-CN.md（中文备份）。英文为主、英链、Typedown 启动慢作为故事开头。
2. `.github/workflows/release.yml`：tag `v*` 触发 → 构建 release → 打 zip → 上传 artifact → 自动创建 GitHub Release。
3. `.gitignore` 完整覆盖：`.trae/skills/`、`.trae/documents/`、历史 `.trae/specs/*/`（保留 `release-and-roadmap/`）、`.skills/`、`docs/review/evidence/` 真机截图、VSCode/JetBrains/DS_Store、Cargo 缓存等。
4. `git status --porcelain` 推 GitHub 前只显示：源码 + docs/ + CHANGELOG + LICENSE + icons + tools + workflows + 保留的 `.trae/specs/release-and-roadmap/`。
5. README 底部"许可未定" → "MIT License" 并指向 LICENSE。

## Non-Goals
- **不**实际 push 到 GitHub（用户说"不需要你操作 GitHub"）
- **不**改写现有 CI gate.yml（只追加 release workflow）
- **不**删除仓库里任何文件，只用 .gitignore 排除（保留本地规划产物、真机截图方便后续走查）
- **不**创建 installer/MSI（release workflow 只产出 zip，单文件 exe 为分发形态）
- **不**写 Typedown 启动慢的技术分析（故事只说"几十秒到数分钟，让人崩溃"）

## Background & Context
- commit `e749ba8` 是当前 HEAD；tag `v1.0.0` 指向 `0943b67`（版本号 1.0.0 对齐）
- 已存在 `.github/workflows/ci.yml`（test + clippy + conformance gate）
- 已建 `LICENSE`（MIT）但 README 底部仍写"许可未定"
- `.gitignore` 当前 8 行：target/.skills/release-bundle/bench results/10MB 语料，远不够
- `.trae/` 目录下有 16 个子目录：7 个历史 specs、3 个 skills、5 个 documents（plan 中间态）、1 个要保留的 release-and-roadmap
- `docs/review/evidence/` 有 25 张 PNG 真机走查截图，累计约 2-3 MB，适合本地走查但不适合 GitHub 仓库体积
- Typedown 是 WinUI 框架实现的 Windows Markdown 编辑器（[github.com/byxiaozhi/Typedown](https://github.com/byxiaozhi/Typedown)），用户最爱的 Markdown 应用，启动需要几十秒到数分钟

## Functional Requirements

### FR-1 双文 README（英文主）
- 创建 `README.md`（英文主），包含 Typedown 背景故事："Veil was born because Typedown, my favorite Markdown editor on Windows, takes tens of seconds to minutes to launch — every time. So I built my own."
- 创建 `README.zh-CN.md`（中文备份），内容与英文 README 对应，**以相对链接指向英文 README**（`[English](README.md)` 在顶部）
- 英文 README 的所有链接（CHANGELOG、LICENSE、Crate、CI badge、架构文档）全部走英文路径或相对路径
- 替换英文 README 底部"License"段的"未定"为"MIT License · see LICENSE"
- `docs/adr/` 目录保留（架构决策是必要文档）；英文 README 的架构部分保留但翻译为英文

### FR-2 Release Workflow
- 创建 `.github/workflows/release.yml`，触发条件：`push.tags` 匹配 `v*`
- Steps：
  1. actions/checkout@v4
  2. dtolnay/rust-toolchain@stable（Windows latest + 组件 clippy）
  3. Swatinem/rust-cache@v2
  4. `cargo build --release -p veil`
  5. `Get-Item target\release\veil.exe | Measure-Object Length` 打印产物大小
  6. 创建 archive zip：veil.exe + veil.pdb + README.md + LICENSE + CHANGELOG.md + icons/veil.ico
  7. softprops/action-gh-release@v2：自动创建 Release，上传 zip，tag message 作为 Release Notes
- Runner：`windows-latest`（唯一平台，Veil 是 Windows 专属）
- 权限：`contents: write`（创建 Release + 上传 artifact）

### FR-3 .gitignore 全面清理
当前 8 行扩展为以下覆盖分组：

| 分组 | 排除项 | 原因 |
|---|---|---|
| IDE / 编辑器 | `.vscode/`, `.idea/`, `*.code-workspace` | 用户本地配置 |
| OS 临时文件 | `.DS_Store`, `Thumbs.db`, `ehthumbs.db`, `Desktop.ini` | macOS/Windows 产物 |
| Git 临时 | `.gitattributes`, `.git/index.lock`, `.git/\*.lock` | 偶尔残留的锁文件 |
| Rust 构建 | `target/`, `**/*.rs.bk` | Cargo 产物（已排除，保留） |
| Trae/AI 中间态 | `.trae/skills/`, `.trae/documents/`, `.trae/specs/drive-v1-excellence/`, `.trae/specs/v1-progress-review-and-roadmap/`, `.trae/specs/own-line-layout-policy/`, `.trae/specs/consolidate-v1-records/`, `.trae/specs/fix-window-minimize-crash/`, `.trae/specs/close-v1-pending-decisions/`, `.trae/specs/finish-v1-release-hardening/` | 规划中间态，保留 `release-and-roadmap/` 作为最终 roadmap |
| Skills 方法论 | `.skills/` | ai-dev-skills 本地实例化，非项目代码 |
| 构建产物 | `release-bundle/`, `release-bundle-windows.zip` | release.ps1 产物 |
| 基准产物 | `bench/results/`, `bench/corpus/pathological-10mb.md` | 本机实测结果 |
| 真机证据 | `docs/review/evidence/**/*.png`, `docs/review/evidence/**/*.jpg` | 25 张走查 PNG，体积大且非必要源码 |
| VS Code 扩展 | `.vscode/extensions.json` | 如果存在 |

明确 **保留**：`.trae/specs/release-and-roadmap/`（v1.1/v2.0 roadmap 是长期规划）、`docs/adr/`、`docs/review/` 中的所有 `.md`（台账和决策）。

### FR-4 GitHub Actions 环境
- 不需要 actionlint 检查（用户没要求）
- 不需要 secrets（release workflow 只用默认 GITHUB_TOKEN）
- 保留 `ci.yml` 不变（只追加 `release.yml`）

## Non-Functional Requirements
- **NFR-1**: 英文 README 行数 ≥ 中文（FR-1 英文版为主）
- **NFR-2**: release workflow 单次执行 ≤ 8 分钟（Windows latest 冷启动 release build ≈ 1.5–2 min，加 artifact ≈ 2 min）
- **NFR-3**: 推 GitHub 后仓库体积增量 ≤ 20 MB（25 张 PNG 不入库后估计 ≤ 5 MB）
- **NFR-4**: `.gitignore` 不包含任何必要源码路径（人工 review）

## Constraints
- **Technical**: release workflow 的 Windows softarchive 行为需要测试（runner 是 GitHub windows-latest）
- **Dependencies**: 无需新增 Rust 依赖；actions/checkout v4, dtolnay/rust-toolchain, softprops/action-gh-release 都是常用 action
- **Business**: 不 push、不创建 GitHub release 远端；只生成本地 workflow 文件

## Assumptions
- 用户 GitHub 仓库名是 `veil`（tag v1.0.0 要对上）
- 用户会自己配置 GitHub Actions 权限（`contents: write` 默认 public repo 给）
- Typedown 的启动慢故事描述用户确认无修改
- `.trae/specs/release-and-roadmap/` 确实要入库（v1.1/v2.0 roadmap 是长期资产）

## Acceptance Criteria

### AC-1: 双文 README 齐全且英文为主
- **Type**: `rule`
- **Given**: 仓库根目录
- **When**: 检查 README.md 和 README.zh-CN.md
- **Then**: 英文 README.md 存在且包含 Typedown 故事（"Typedown" + "tens of seconds" + "built my own" 三个关键词命中）；中文 README.zh-CN.md 存在且相对链接指向英文；英文 README License 段写 MIT
- **Pass Condition**: 三个 grep 全中 + Test-Path 返回 True
- **Evidence**: Select-String 输出

### AC-2: Release Workflow 可解析且逻辑正确
- **Type**: `rule`
- **Given**: `.github/workflows/release.yml`
- **When**: 人工读 + yamllint（如果有）
- **Then**: `on.push.tags = v*`；Windows latest runner；steps 包含 checkout → toolchain → build → zip → gh-release；permissions 含 contents: write
- **Pass Condition**: 5 个关键字段全中
- **Evidence**: 人工审阅输出

### AC-3: .gitignore 覆盖完整
- **Type**: `rubric`
- **Dimension**: 覆盖度（IDE/OS/构建/中间态/真机证据 5 类均覆盖）
- **Scale**: 1-5
- **Anchors**: 1=只覆盖 target；3=覆盖 target/.skills/.vscode；5=覆盖全部 5 类 + 明确保留 release-and-roadmap
- **Pass Threshold**: >= 4
- **Evidence**: `.gitignore` 内容 + `git status --porcelain` 验证

### AC-4: 推 GitHub 前只显示必要文件
- **Type**: `rule`
- **Given**: 新 `.gitignore`
- **When**: `git status --porcelain` 在当前工作区执行
- **Then**: 输出不包含 `.trae/skills/`、`.trae/documents/`、`.trae/specs/` 下历史 specs、`.skills/`、`docs/review/evidence/**/*.png`、`release-bundle/` 等排除项；保留 `.trae/specs/release-and-roadmap/`、`docs/adr/`、`docs/review/**/*.md`
- **Pass Condition**: 被排除项 grep 不到；被保留项 grep 得到
- **Evidence**: git status 输出

### AC-5: Release workflow 产物大小验证
- **Type**: `rule`
- **Given**: GitHub Actions runner（本地可模拟）
- **When**: 执行 release workflow step "Get-Item target\release\veil.exe"
- **Then**: veil.exe 大小在 4–8 MiB（本地已验证 5.58 MiB，GitHub runner 应该一致）
- **Pass Condition**: 本地模拟构建产物落在区间
- **Evidence**: PowerShell 脚本输出
