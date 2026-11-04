# Veil v1 进度 Review 与路线图 — Implementation Plan

---

## 任务 1：进度快照文档化

- **Status**: `completed`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - 已在 spec.md Background & Context 章节完整列出 M1-M13 共 13 个已完成里程碑
  - 已在门禁快照表列出 10 项数字红线实测值
  - 已在已决策不做项表列出 8 项排除项
- **Acceptance Criteria Addressed**: AC-1
- **Test Requirements**:
  - `rule` TR-1.1: 13/13 里程碑有 commit hash 或文件路径可追溯 → PASS（M1-M13 全部已在 spec.md 列出）
  - `rule` TR-1.2: 门禁数字与 `cargo test --workspace` / `cargo clippy` / `cargo build --release` 实测一致 → PASS（2026-11-04 实测 875/0/12, clippy 0, 5,825,536 B）
- **Completion Evidence**:
  - spec.md Background & Context 章节
  - `cargo test --workspace` → 875 passed / 0 failed / 12 ignored
  - `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
  - `target\release\veil.exe` → 5,825,536 B ≈ 5.56 MiB

---

## 任务 2：走查清单 50 项优先级分区

- **Status**: `completed`（分区完成；真机回填为任务 6）
- **Priority**: high
- **Depends On**: 任务 1
- **Description**:
  - 将 C-H 区 50 项 `needs-user` 按四档优先级分类
  - 分区依据：(1) 是否影响 v1 基本写作闭环 (2) 自动化覆盖程度 (3) 发布门槛硬性约束
  - 优先级定义：
    - **P0 阻断**：无自动化覆盖 + 影响 v1 核心承诺 + 必须真机验证才能发布
    - **P1 高**：有部分/完全自动化覆盖但仍需真机验证正确性
    - **P2 中**：有完全自动化覆盖 + 非核心路径 + 发布门槛不强制要求真机通过
    - **P3 低**：平台依赖项（无真机环境无法测）或已决策降级

### 完整优先级分区表

#### C. 排版与中日韩细节（11 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| C1 | 中英混排自动间距 | P1 | 完全（3 条：cjk_to_ascii / ascii_to_cjk / both_boundaries） | 有测试但排版视觉效果需目检确认无违和 |
| C2 | 行首/行尾禁则 | P1 | 完全（2 条：forbidden_chars / open_brackets_never_end） | 禁则逻辑正确但 DPI/窗口宽度边缘情况需真机 |
| C3 | 满行悬挂标点 | P1 | 完全（2 条：hanging_punctuation / kinsoku_glue） | 同 C2 |
| C4 | 各标记层级与行高稳定 | P1 | 部分（2 条：heading_layout / reveal_hides_strong_markers） | 有布局测试但"层级稳定"需真机目检 |
| C5 | 表格点入编辑/Tab跳格/行列增删/对齐 | P1 | 完全（4 条：table_renders / table_shows_grid / alignment_offsets / tab_advances） | 有测试但表格交互路径真机验证 |
| C6 | 任务列表复选框点击切换 | P1 | 完全（1 条：task_checkbox_click） | 逻辑正确，真机确认点击命中区域 |
| C7 | 图片粘贴/拖入/资源管理器文件列表 | P1 | 完全（3 条：paste_image / paste_image_files / paste_image_requires） | 三条入口均有测试，真机确认落盘路径 |
| C8 | 公式位图/上下标/分式/滚动不闪 | P1 | 完全（3 条：math_block_renders / inline_math_renders / math_render_failure） | 有渲染测试但位图质量 + 滚动闪烁需真机 |
| C9 | 源码标记随光标进出展开/收起 | P1 | 完全（3 条：list_bullet_toggles / blockquote_line_reveal / thematic_break_elides） | 有显隐测试，真机确认光标进出无行高跳 |
| C10 | 10 万字文档打开 ≤2s/滚动跟手/内存 ≤150MB | P0 | **无** | 红线指标，无自动化覆盖，必须真机实测 |
| C11 | 超长单行不崩溃/降级提示/源码完整 | P1 | 完全（3 条：oversized_single_block / oversized_block_tail / oversized_line_count） | 有硬上限测试，真机确认降级提示可读性 |

#### D. 3c 新增交互（14 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| D1 | Ctrl+滚轮缩放档位与状态栏提示 | P1 | 完全（2 条：zoom_step / zoom_by_accumulates） | 有档位/提示测试，真机确认滚动条同步 |
| D2 | 触控板捏合缩放灵敏度 | P2 | 部分（1 条同 D1） | 依赖触控板硬件，D1 通过则 D2 大概率通过 |
| D3 | Ctrl+= / Ctrl+- 档位与上下限 | P1 | 完全（2 条：zoom_quantizes / zoom_bindings） | 边界 clamp 逻辑测试，真机确认长按不越界 |
| D4 | Ctrl+0 精确还原 100% | P1 | 完全（2 条：zoom_step_roundtrips / zoom_bindings） | roundtrip 测试，真机确认精确 |
| D5 | 缩放后点击落点与滚动钳位 | P1 | 完全（1 条：zoom_shrink_clamps） | 钳位逻辑测试，真机确认点击落点 |
| D6 | 200% DPI 下缩放复测 | P2 | 部分（1 条模拟 DPI 2.0） | 依赖 200% DPI 硬件，模拟测试已覆盖核心逻辑 |
| D7 | F11 无边框全屏 | P1 | 部分（2 条：fullscreen_headless / fullscreen_binding） | headless 只验证逻辑，真机确认盖住任务栏 |
| D8 | 全屏下 Esc 只关菜单 | P1 | 部分（1 条同 D7） | 按键语义测试，真机确认全屏不退出 |
| D9 | Ctrl+B 包裹/解包裹/单撤销步 | P1 | 完全（2 条：bold_wraps / bold_unwraps） | 包裹逻辑测试，真机确认单撤销步 |
| D10 | 空选区 Ctrl+B 标记对与连按取消 | P1 | 完全（1 条：empty_selection_inserts_pair） | 逻辑测试，真机确认光标位置 |
| D11 | Ctrl+I / Ctrl+K 行内格式与光标落点 | P1 | 完全（2 条：italic_on_cjk / link_wraps_selection） | 逻辑测试，真机确认链接光标落点 |
| D12 | `**粗体**` 内 Ctrl+I 不误拆 | P1 | 完全（2 条：italic_on_bold_stacks / italic_on_cjk_bold） | 嵌套逻辑测试，真机确认结构正确 |
| D13 | 视图菜单缩放/全屏与快捷键提示 | P2 | 完全（2 条：zoom_menu / menu_definitions） | 菜单定义测试，D1-D7 通过则 D13 自动通过 |
| D14 | 纯视图操作不改文档/不置脏 | P1 | 完全（2 条：zoom_is_view_only / fullscreen_headless） | 脏标不变测试，真机确认 |

#### E. 打印 / 导出 PDF（8 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| E1 | 打印对话框弹出 | P2 | 部分（仅验证打印端口调用） | 系统对话框走平台 FFI，逻辑正确即可 |
| E2 | Microsoft Print to PDF 保存成功 | P2 | **无** | 依赖系统 PDF 驱动，用户有则测无则降级 |
| E3 | 多页内容连续/无重字截字/页边距均匀 | P0 | 部分（2 条：print_paginates / print_breaks_inside） | 分页断点测试有，但 PDF 渲染质量需真机 |
| E4 | 公式/表格跨页硬切/不重叠不漏 | P1 | 完全（1 条：print_hard_cuts_atomic_block） | 硬切逻辑测试，真机确认跨页效果 |
| E5 | 页码范围打印 | P3 | **无** | 系统对话框功能，FFI 调用正确即可，硬件依赖 |
| E6 | 物理打印机抽验 | P3 | **无** | 硬件依赖，无物理打印机则降级 |
| E7 | 放大到 150% 后打印版式不变 | P1 | 完全（1 条：zoom_ignored_during_print） | 缩放隔离测试，真机确认版式一致 |
| E8 | 打印过程无闪烁/残留遮罩/回编辑态 | P1 | 部分（1 条：print_paginates） | 状态复原测试，真机确认视觉 |

#### F. 主题与设置（4 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| F1 | 浅/深/跟随系统即时切换与「当前」标记 | P1 | 完全（2 条：switching_theme / theme_menu_marks） | 切换 + 菜单标记测试，真机确认视觉同步 |
| F2 | 深色重启保持/config.toml 一致 | P1 | 完全（2 条：startup_applies_config / switching_theme_persists） | 持久化测试，真机确认重启后 config |
| F3 | 跟随系统时系统切深浅即时生效 | P1 | 完全（1 条：system_mode_follows_live_switch） | 注册表监听测试，真机确认系统切换后应用跟随 |
| F4 | Win11 Mica / Win10 不崩 | P3 | **无** | DWM/Mica 平台实现，无单测。Win11 看视觉效果即可，Win10 不测 |

#### G. 数据安全（9 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| G1 | 任务管理器杀进程后重启恢复 | P1 | **无** | 进程级场景，恢复读写单测覆盖但杀进程恢复是物理场景 |
| G2 | 恢复后 Ctrl+S 保存并清 recovery.dat | P1 | **无** | 同 G1，文件系统物理操作 |
| G3 | 外部修改 1s 内提示并挂起自动保存 | P1 | 完全（1 条：external_change_notifies） | 文件监听测试，真机确认通知延迟 |
| G4 | Ctrl+S 先备份外部版本再覆盖 | P1 | 完全（2 条：manual_save_conflict_backs_up / failed_backup_aborts） | 备份逻辑测试，真机确认 .bak 文件生成 |
| G5 | 外部删除后提示并重建文件 | P1 | 完全（1 条：external_delete_notifies） | 重建逻辑测试，真机确认文件重建 |
| G6 | 无路径草稿 Ctrl+S 提示另存为 | P2 | 部分（1 条 save_draft_without_path 只验证不写盘） | 提示文案需真机 |
| G7 | 关窗/退出三选一/清快照 | P1 | 完全（2 条：close_confirmation / clean_close） | 三选一逻辑 + 快照清理测试，真机确认退出路径 |
| G8 | Ctrl+N 未保存三选一 | P1 | 完全（3 条：new_draft_refuses / untitled_save_choice_cancelled / untitled_save_choice_saves_as） | 三选一逻辑测试，真机确认 Ctrl+N 路径 |
| G9 | Ctrl+O / Ctrl+Shift+P 快速打开与只读 | P1 | 完全（2 条：quick_open_lists / quick_open_refuses） | 面板列出 + Esc 关闭测试，真机确认只读状态 |

#### H. 菜单与键盘可达性（4 项）

| # | 项目 | 优先级 | 自动化覆盖 | 分区依据 |
|---|---|---|---|---|
| H1 | 三个菜单项数/灰显状态 | P1 | 完全（2 条：menu_definitions / paint_draws_menu） | 定义 + 绘制测试，真机确认项数与灰显 |
| H2 | 菜单打开后 ↑↓/Enter/Esc | P3 | 无 | **已决策降级**：v1 不承诺键盘导航（commit 9a25d73） |
| H3 | 点击菜单外部只关菜单/不动光标 | P1 | 完全（1 条：click_outside_open_menu） | 关闭逻辑测试，真机确认光标不动 |
| H4 | 右键菜单三分类正确 | P1 | 部分（3 条：items_cover_three_contexts / layout_clamps / hit_round_trips） | 分类集合 + 命中测试，真机确认右键目标判定 |

### 分区统计

| 优先级 | 数量 | 说明 |
|---|---|---|
| **P0 阻断** | 2 | C10（10万字打开）、E3（多页打印无截字） — 无自动化覆盖且红线指标 |
| **P1 高** | 38 | 有自动化覆盖但需真机确认视觉/交互正确性 |
| **P2 中** | 6 | 有完全覆盖 + 非核心路径 |
| **P3 低** | 4 | 平台依赖 / 硬件依赖 / 已决策降级（H2, E5, E6, F4） |

---

## 任务 3：触发条件与非目标边界汇总

- **Status**: `completed`
- **Priority**: high
- **Depends On**: 任务 1
- **Description**:
  - 汇总批次 3 触发式清单 6 项 + ADR-011 四条触发条件 + 已决策不做项 8 项
  - 所有项标注阈值与决策来源

### 批次 3 触发式清单总表

| # | 项 | 现状 | 触发条件 / 说明 |
|---|---|---|---|
| 1 | Events 契约 | **已实施** | 索引层优化（Block 字节区间 + 几何复用）已使击键 p95 从 13.8ms → 5.2ms（10.5k 块） |
| 2 | Block 行号→字节 | **已实施** | Block.range + Session::blocks() 已消除 rope 换算 |
| 3 | undo 断组入口 | **已实施** | Session::break_undo_group() 已在光标移动/扩选/鼠标点击处接线 |
| 4 | Transaction 简化 | **已实施** | 收敛为单个 PrimEdit |
| 5 | Mark 排序不变量 | **已解决** | commit a3b7062：删除不变量 + create_mark 改 push |
| 6 | line_col | **仅改文档口径** | 列按码点计数；CRLF 计一列。不改内核 |

### ADR-011 wgpu 触发条件（4 条）

| # | 触发条件 | 当前状态 | 来源 |
|---|---|---|---|
| 1 | 4K/120Hz 大面积滚动掉帧 + profiler 显示 CPU（shaping/光栅/合成）为主因 | 未触发（当前 10.5k 块 p95 5.2ms 足够） | ADR-011 L37 |
| 2 | 内容区需要 Mica/亚克力（不只是标题栏区域） | 未触发（当前 DWM 标题栏 Mica + 内容区纯色兜底） | ADR-011 L38 |
| 3 | 需要文档级过渡动画 + 软件合成无法达到目标帧率 | 未触发（v1 无动画需求） | ADR-011 L39 |
| 4 | 需要 GPU 侧公式/图片实时缩放 | 未触发（CPU 光栅缓存命中，实测脏块排版+光栅 0.4ms） | ADR-011 L40 |

### 已决策不做项总表

| # | 项 | 决策来源 | 理由 |
|---|---|---|---|
| N1 | H2 菜单 ↑↓/Enter 键盘导航 | commit 9a25d73 | v1 承诺 Esc + 快捷键直达，导航 v2 |
| N2 | I3 表格内 Ctrl+I 渲染 | walkthrough-results L140 | shape_cell 纯文本，富文本单元格 v2 |
| N3 | wgpu GPU 渲染 | ADR-011 | 体积 + 依赖硬约束 |
| N4 | build_refs 增量 | remediation_plan_v2 L535-537 | first-wins 语义风险 |
| N5 | Events 增量（数据扩展） | remediation_plan_v2 L618 | 已实施（不是不做） |
| N6 | 200ms 撤销静默窗口 | commit fa584ed | 时间窗使撤销语义不确定 |
| N7 | vello / skia / 自研 GPU | ADR-011 | 同 wgpu 理由 |
| N8 | 源码模式（独立视图） | INSTANCE §0 | Reveal 已替代 |

---

## 任务 4：v1 发布路径制定

- **Status**: `completed`
- **Priority**: high
- **Depends On**: 任务 2, 任务 3
- **Description**:
  - 明确从当前状态到 `git tag v1.0.0` 的最短路径
  - 列出必须真机回填的条目、可降级的条目、明确进入 v2 的条目

### v1 发布路径（最短）

#### Step 1：P0 阻断项必须真机验证（2 项）
| # | 项 | 方法 | 通过标准 |
|---|---|---|---|
| C10 | 10 万字文档打开 ≤2s / 内存 ≤150MB | 启动 `03-10万字.txt` → 任务管理器看私有工作集 + 秒表 | 打开墙钟 ≤2s，内存 ≤150MB |
| E3 | 多页打印无重字截字 | `02-混合元素.md` → Microsoft Print to PDF → 打开 PDF 目视 | 无重字/截字，页边距均匀 |

#### Step 2：P1 高优先级批量真机验证（38 项）
- 按 C→D→E→F→G→H 顺序，每区 30 分钟内可完成
- 执行协议见 `3e-manual-walkthrough.md` §执行协议
- 不通过项记录严重度 + 复现步骤 + 截图 → 立项修复

#### Step 3：P2/P3 降级处理（10 项）
| # | 项 | 降级方案 |
|---|---|---|
| D2 | 触控板捏合灵敏度 | 有 D1 Ctrl+滚轮通过即可，触控板路径复用同一逻辑 |
| D6 | 200% DPI 缩放 | 无 200% DPI 硬件 → 模拟 DPI 2.0 测试通过 + 注明 |
| D13 | 视图菜单快捷键提示 | D1-D7 通过则自动覆盖 |
| E1 | 打印对话框 | FFI 调用成功即可 |
| E2 | PDF 保存 | 用户有驱动则测，无则降级 |
| E5 | 页码范围 | 依赖系统对话框，降级 |
| E6 | 物理打印机 | 硬件依赖，降级 |
| F4 | Mica / Win10 | Win11 看视觉，Win10 不测 |
| H2 | 菜单键盘导航 | **已决策降级 v2** |
| I 盲测 | 盲测前置 | 可延后，非 tag 阻断项 |

#### Step 4：打 tag
```powershell
git tag -a v1.0.0 -m "Veil v1.0.0 — Markdown 编辑器首版发布"
```

#### Step 5：Release Notes
- 依据：README 数字红线表 + `3e-walkthrough-results.md` 实测记录
- 不写内部架构机制（§违禁项清单）

---

## 任务 5：代码缺陷修复立项（条件性）

- **Status**: `pending`（等待真机走查结果）
- **Priority**: high
- **Depends On**: 任务 4 Step 1-2 真机走查结果
- **Description**:
  - 真机走查过程中发现的阻断性缺陷，逐项立项修复
  - 修复每项缺陷的通用流程：根因定位 → 修复 → 新增回归测试 → cargo test + clippy → 独立 commit
- **已知可能的代码修复方向**（供真机发现问题时参考）：
  - C10 性能问题：如果打开超时，需 reparse 冷启动路径分析
  - E3 分页截字：如果 PDF 有截字，需分页引擎 `break_inside_tall_block` 路径
  - 任何其他 P0/P1 项的真机缺陷
- **Acceptance Criteria Addressed**: AC-5
- **Test Requirements**:
  - `rule` TR-5.1: 每个修复有对应回归测试 → PASS（测试绿）
  - `rule` TR-5.2: cargo test --workspace 全绿 + clippy 零警告 → PASS

---

## 任务 6：真机走查回填

- **Status**: `pending`（用户执行）
- **Priority**: high
- **Depends On**: 任务 4
- **Description**:
  - 用户本人执行 release build 真机走查
  - 启动命令：`cargo build --release -p veil && .\target\release\veil.exe "$env:TEMP\veil-walkthrough\02-混合元素.md"`
  - 跑前清空 `%APPDATA%\Veil\`
  - 结果直接回填 `docs\review\3e-walkthrough-results.md` 对应行
  - 素材：`%TEMP%\veil-walkthrough\01-长文1万字.md`、`02-混合元素.md`、`03-10万字.txt`
- **Acceptance Criteria Addressed**: AC-2（需要真机结果最终确认分区合理性）、AC-4
- **Test Requirements**:
  - `rule` TR-6.1: 50 项 needs-user 全部回填（通过/不通过/阻断），无未标注
  - `rule` TR-6.2: 阻断项 = 0 → 可 tag；否则立项修复后再回填

---

## 全局约束

- **门禁不变**：任何代码修改必须保持 875 tests / 0 failed / clippy 0 / ≤15 MiB 体积
- **一个切片一 commit**：每个独立修复一个 commit
- **无新 crate 依赖**：批次 3 触发式项和批次 2 都未引入新依赖
- **零防御层**：崩溃修复哲学——正确建模而非 catch_unwind
