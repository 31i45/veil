# 首帧阶段证据（测量先行 · 优化后复测）

- 口径：`VEIL_PERF_TRACE=1` 下每阶段**累计**毫秒（同一 `PERF_START` 原点），release 构建，约 10 万字 Markdown（99,543 字符）
- 样本：5 次独立实例，取中位
- 复现：

```powershell
cargo build --release --bin veil
$env:VEIL_PERF_TRACE=1
& .\target\release\veil.exe "$env:TEMP\veil-perf\100k.md" 2>&1 | Select-String 'veil-(stage|first-frame)-ms'
```

## 结论摘要

| 时点 | 端到端首帧中位 | 说明 |
|---|---:|---|
| 本轮起点（Task 1 采集，机器高负载） | 428.2 ms | 含埋点自身 ~48 ms 同步写 stderr 的虚增 |
| 字体按需加载后 | 167.9 ms | `app-fonts` 136.7 → 12.2 ms |
| 启动 IO 后移后 | 161.1 ms | `app-new` 尾部 8.9 → 4.1 ms |
| 埋点异步化（消除虚增）后 | 102.5 ms | **真实值**；同步 `eprintln!` 每条约 4.4 ms |
| 窗口外观效果后移后 | 77.7 ms | 简单语料（标题 + 段落） |
| 语法高亮预热后 | 83.9 ms | 门禁语料（含 520 个 ```rust 围栏） |
| **门禁脚本终判** | **96.5 ms** | `perf_gate.ps1` 5 次：81.3 / 138.9 / 96.5 / 81.6 / 103.3，**PASS（余量 3.5 ms）** |
| 门禁脚本（清理前） | 96.5 ms | 5 次：81.3 / 138.9 / 96.5 / 81.6 / 103.3，PASS（余量 3.5 ms） |
| 埋点开关缓存 + 移除临时子阶段后 | **81.9 ms** | 7 次门禁语料复测：77.4–91.5 ms |
| **门禁脚本终判（最终）** | **80.7 ms** | 5 次：80.7 / 83.5 / 78.0 / 85.6 / 77.6，**PASS（余量 19.3 ms）** |

## 环境敏感性（必读，诚实记录）

同一二进制、同一门禁语料，在不同机器负载下差异显著：

| 条件 | 样本 | 中位 | 区间 |
|---|---|---:|---|
| 空载（无并行编译） | 9 次 | 78.4 ms | 76.9 – 89.3 ms |
| 空载复测（清理后） | 7 次 | 81.9 ms | 77.4 – 91.5 ms |
| 门禁脚本测量阶段 | 5 次 | 96.5 ms | 81.3 – 138.9 ms |
| 门禁脚本终判 | 5 次 | **80.7 ms** | 77.6 – 85.6 ms |
| **持续编译负载下**（独立审查员复测） | 8 次 | **114.9 ms** | 106.2 – 175.9 ms |

结论：
- **门禁协议下（测量阶段无并行编译）稳定达标**，中位 78–97 ms。
- **机器处于持续 CPU 争用（如并行 `cargo clippy` / `cargo test`）时会超过 100 ms**，此时 `window-created` 从 ~27 ms 涨到 93–162 ms、`app-editor` 从 ~24 ms 涨到 ~38 ms——几乎所有阶段同比例变慢，是抢占而非代码回归。
- 因此「首帧 < 100 ms」的结论带明确前提：**单实例、无并行重编译负载**。门禁脚本的测量段满足该前提；日常使用（用户在编辑器里降速的窗口期通常无编译负载）也满足。这一点已写入证据，不做掩饰。
- 余量在轻载下 10–20 ms、在门禁脚本自身的测量条件下 3.5 ms，属偏薄但真实。

> 关键教训：**测量工具污染了被测对象**。stage 埋点若同步写 stderr，每条约 4.4 ms，12 条 = ~48 ms 虚增，足以让任何优化看起来无效。异步写线程修好后，真实首帧才显形。
>
> 第二课：**语料决定归因**。同一二进制在「标题 + 段落」语料上 render 段仅 12.4 ms，在含 520 个 Rust 围栏的门禁语料上是 56.6 ms——40 ms 来自 syntect 首次 `parse_line` 的正则惰性编译。只看简单语料会漏掉这个瓶颈。
>
> 波动说明：门禁 5 次里有 1 次 138.9 ms（机器负载抖动），其余 81–103 ms。余量 3.5 ms 偏薄，属已知风险，已如实记录。

## 最终阶段表（门禁语料，5 次中位，真实值）

| 阶段 | 阶段耗时 ms | 预算 | 判定 |
|---|---:|---:|---|
| 入口 → `doc-load` | 1.7 | ≤20（入口→文档） | PASS |
| `app-config` | 6.1–8.0 | — | — |
| `app-theme` | ~0.1 | — | — |
| `app-startup-doc` | ~0.5 | — | — |
| `app-editor`（Markdown 解析） | 18.3–23.6 | — | — |
| `app-fonts`（字体系统） | 7.2–12.5 | — | — |
| `app-index`（块枚举 + 估算几何） | 2.7–6.9 | — | — |
| `app-new` 尾部 | ~0.1 | — | — |
| `App::new` 合计 | ~34.8–38.8 | ≤35 | 临界（见下） |
| 窗口创建 + surface | 26.8 | ≤25 | 轻微超预算（见下） |
| 首次 render + present（含代码块） | 13.8–23.9 | ≤20 | PASS（语法预热后） |
| **端到端首帧** | **83.9–96.5** | ≤100 | **PASS** |

## 超预算项说明（如实标注）

### 窗口段 26.8 ms > 25 ms

二级切分（临时埋点，已移除）表明该段由两部分主导，均**不可压缩**：

| 子项 | ms | 可压缩性 |
|---|---:|---|
| winit `event_loop.create_window` | ~18 | 不可压缩（OS 建窗 + winit 后端初始化） |
| softbuffer `Context::new` + `Surface::new` | ~9 | 不可压缩（首帧必须 present） |
| DWM 效果 / 擦除色 / 动画开关 | ~0.1 | 已后移到 `set_visible(true)` 之前 |
| 显示器枚举 / 工作区 / 图标 | <1 | 已有保存几何时跳过工作区查询 |

26.8 ms 与 25 ms 的差额落在机器噪声范围内（交叉 ABBA 实测 ±2 ms），且构成已证明为不可压缩。**未修改预算或阈值。**

### `App::new` 34.8 ms ≈ 预算上限 35 ms

其中 Markdown 解析 18.3 ms 为最大项，是首帧内容所必需（可见块需要真实结构）；字体系统已从 136.7 ms 压到 7.2 ms。该段未再压缩。

## 前两次优化为何无效（复盘保留）

`render()` 内 `ensure_layouts_visible` + `recompute_geometry` + 表格/公式 ensure 合计仅约 6.6 ms。因此上一轮「移除 build 期全量几何累计」「视口外块用估算高度」只作用于 1.5% 的池子，天花板就是 6.6 ms。真正的池子是字体系统（136.7 ms）与窗口创建（181 ms 原始）——本轮证据先行才定位到。

## 附带修复的真实缺陷

估算几何下 `MoveDocEnd` 后真实重算会把图片块从 22px 展开到数百 px，导致光标掉出视口（`new_paragraph_caret_with_cold_images_below` 长期失败）。已修：几何重算前后比较光标可见性，仅在「由可见变不可见」时回滚滚动，用户主动滚走不会被拽回（commit `ecfdcef`）。

## 本轮 commit 序列（均本地，未 push）

```
a24191d chore(perf): cache trace switch and drop ad-hoc render stages
ae850a5 perf(render): start syntax prewarm at process entry
1d2bbf4 perf(render): cut first-frame raster cost for code blocks
0d82aa2 perf(window): apply native effects just before first show
adf069d fix(perf): unblock stderr from first-frame path
ecfdcef fix(scroll): keep caret visible across geometry recompute
dcea88e perf(startup): defer non-critical startup work past first frame
fa967ee perf(startup): skip work-area query when saved window geometry exists
bd3ba7b perf(fonts): load only required font faces at startup
ce2695f chore(perf): stage-level first-frame instrumentation
```

## 门禁终态（如实记录）

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --exclude veil-core -- -D warnings` | PASS |
| `cargo test --workspace` | PASS（全绿；veil-ui 1423 passed） |
| `cargo build --release --bin veil` | PASS |
| 端到端首帧 < 100 ms | **PASS（80.7 ms，余量 19.3 ms）** |
| 击键 p99 < 16 ms | PASS（0.305 ms） |
| 常驻内存 < 300 MB | PASS（65.47 MB） |
| 体积 ≤ 14,680,064 B | PASS（14,166,528 B） |
| 压力基准「5005 块全量排版 < 100 ms」 | **FAIL（289.73 ms）** —— 口径未改、阈值未动，如实保留 |

压力基准说明：该指标为进程内 `Session::open` 后把全部 5005 块各排版一次的中位耗时，**不含**进程启动、窗口创建与 present，不是用户可感知延迟。本轮未调整其判定口径（按 spec 约定留待产品决策）。
