# Veil v1.0 Release Packaging + Roadmap - Independent Review

- [x] CP-R1: 版本号全链一致（Cargo.toml = exe FileVersion = tag）
  - **Type**: `rule`
  - **Covers**: AC-1, TR-1.1, TR-3.1
  - **Evidence**: git show v1.0.0:Cargo.toml → version = "1.0.0"; veil.exe FileVersion/ProductVersion = 1.0.0

- [x] CP-R2: 仓库规范文件齐全（LICENSE + CHANGELOG + icons）
  - **Type**: `rule`
  - **Covers**: AC-2, TR-1.2, TR-2.1, TR-2.2, TR-2.3
  - **Evidence**: LICENSE (MIT); CHANGELOG.md (## v1.0.0 — 2026-11-05); icons/veil.ico (32054 B); README.md 第 3 行引用 icons/veil.ico

- [x] CP-R3: release.ps1 一键脚本通过
  - **Type**: `rule`
  - **Covers**: AC-3, TR-4.1, TR-4.2, TR-4.3
  - **Evidence**: 执行 exit 0, 7 步全部通过, 7 文件归档至 release-bundle/veil-v1.0.0/, 耗时 69.8s

- [x] CP-R4: release 构建产物合格
  - **Type**: `rule`
  - **Covers**: AC-4, TR-3.2, TR-3.3
  - **Evidence**: veil.exe 5.58 MiB（4-8 MiB 区间）, FileDescription="Veil Markdown 编辑器"

- [x] CP-U1: v1.1 roadmap 条目覆盖度与可验收性
  - **Type**: `rubric`
  - **Covers**: AC-5, TR-5.1
  - **Scale**: 1-5; Anchors 1/3/5 as in spec
  - **Pass Threshold**: >= 4
  - **Score**: 5/5 (修复后)
  - **Rationale**: 7 列齐全（#/条目/优先级/触发条件/验收标准/工作量/依赖前置），4 批次 17 条目，门禁 6 条，时间线完整。修复后触发条件与优先级独立列齐全
  - **Evidence**: v1.1-roadmap.md 批次 A-D 表格

- [x] CP-U2: v2.0 roadmap 架构决策与触发条件明确度
  - **Type**: `rubric`
  - **Covers**: AC-6, TR-6.1
  - **Scale**: 1-5; Anchors 1/3/5 as in spec
  - **Pass Threshold**: >= 4
  - **Score**: 5/5 (修复后)
  - **Rationale**: 永恒原则首节 + 协作永久排除 + wgpu 四条件硬数值（p95≥15ms/内存≥120MB/FPS<55/渲染占比≥40%）+ 依赖前置 + ADR/Spec 列引用 remediation plan。修复后架构批次 A1-A5 有 ADR 引用
  - **Evidence**: v2.0-roadmap.md 永恒原则 + 渲染后移 + 功能新增

- [x] CP-R5: 永恒设计原则已写入 spec + roadmap
  - **Type**: `rule`
  - **Covers**: AC-5, AC-6
  - **Evidence**: spec.md 第 33 行 + v2.0-roadmap.md 首节 + v1.1-roadmap.md 首节，三处文案完全一致

## Review History

### Review R1 (2026-11-05)
- **Result**: `pass`
- **Checks Performed**: 版本号全链验证, 规范文件存在性, release.ps1 7 步输出, 产物大小/属性, v1.1/v2.0 roadmap 评审
- **All 7 checkpoints pass; rubric 5/5 和 5/5（修复后）**
- **Findings (修复)**:
  - F-01 (medium): v1.1 roadmap 无触发条件列 → 修复：新增 7 列含触发条件
  - F-02 (low): v1.1 roadmap 无独立优先级列 → 修复：每批次标注 P1-P3
  - F-03 (low): v2.0 roadmap ADR/Spec 列全空 → 修复：补 "remediation plan v2 批次 3"
- **Final git log**:
  - e749ba8 docs(roadmap): add priority/trigger columns, ADR refs (修复 findings)
  - 0943b67 tag v1.0.0: 版本号对齐 + LICENSE + CHANGELOG + release.ps1 + roadmaps
  - 4847646: 真机归档 + needs-user 清零 + 门禁六条件全过
