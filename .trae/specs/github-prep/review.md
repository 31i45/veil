# Veil GitHub Prep - Independent Review

- [x] CP-R1: README.md (English) contains Typedown origin story + MIT license + zh-CN link
  - **Type**: `rule`
  - **Covers**: AC-1, TR-1.1/1.2/1.3/1.4
  - **Evidence**: findstr 三条全中 — "Typedown" + "tens of seconds" + "MIT License" + "README.zh-CN.md"

- [x] CP-R2: README.zh-CN.md exists with English back-link + Typedown story
  - **Type**: `rule`
  - **Covers**: AC-1, TR-2.1/2.2/2.3
  - **Evidence**: Test-Path True; Typedown 故事段中文完整; 顶部 "[English](README.md)" 链接存在

- [x] CP-R3: release.yml has correct tag trigger + windows runner + gh-release step
  - **Type**: `rule`
  - **Covers**: AC-2, TR-3.1/3.2/3.3/3.4
  - **Evidence**: (a) on.push.tags 'v*', (b) windows-latest, (c) softprops/action-gh-release@v2, (d) contents: write

- [x] CP-R4: .gitignore covers 8 groups + explicitly preserves release-and-roadmap
  - **Type**: `rubric`
  - **Covers**: AC-3, TR-4.1/4.2/4.3/4.4
  - **Score**: 5/5
  - **Evidence**: IDE + OS + Git locks + Rust/Cargo + Trae intermediates + Skills + Build/release + Bench + Real-device PNG = 8 组全覆盖; 两条 `!` 反忽略保留 `.trae/specs/release-and-roadmap/`

- [x] CP-R5: Local simulation builds 6-item zip in 4-8 MiB
  - **Type**: `rule`
  - **Covers**: AC-5, TR-5.1/5.2
  - **Evidence**: zip 6 项 (veil.exe + veil.pdb + README.md + LICENSE + CHANGELOG.md + icons/veil.ico); exe 5.58 MiB (4-8 MiB 区间)

- [x] CP-R6: cargo clippy zero warnings
  - **Type**: `rule`
  - **Covers**: TR-6.1
  - **Evidence**: exit code 0, "Finished dev profile", 零警告

- [x] CP-R7: git status shows only intended files
  - **Type**: `rule`
  - **Covers**: AC-4, TR-4.1
  - **Evidence**: 被排除项 (.trae/skills/, .trae/documents/, .trae/specs/drive-v1-excellence/, .skills/, release-bundle/, docs/review/evidence/*.png) 均未出现

## Review History

### Review R1 (2026-11-05)
- **Result**: `pass`
- **Checks**: 7/7 pass; rubric CP-R4 score 5/5
- **Findings**: 无 actionable issues
- **Remaining note**: release.yml 未在 GitHub Actions 实际跑过（本地无法模拟 windows-latest），但 YAML 结构与 spec 完全一致，第一次推 GitHub 打 tag v1.0.1 会自然触发
