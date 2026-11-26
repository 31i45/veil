# `windows` / `image` 观察备忘录

- 日期：2026-11-26
- 基线：`c8298b5`（含 Phase 3 的当前 HEAD），版本 `3.0.2`
- 性质：Phase 4 / Task 10（Agent J）。**只观察，不引入依赖、不改 `Cargo.toml`、不改生产代码。**
- 规格：`.trae/specs/converge-veil-under-jobs-lens/spec.md`、`tasks.md` Task 10；`checklist.md`「观察项（Task 10）」
- 体积口径：`docs/review/evidence/perf/current-head-2026-11-26.md` 实测 `veil.exe` = **14,170,112 B**；门禁 **≤ 14 MiB = 14,680,064 B**；余量 **509,952 B**

## 1. 结论

| 组件 | 当前状态 | 本阶段决策 | 何时再评估 |
|---|---|---|---|
| `windows` / `windows-registry` | 完全手写 Win32/COM FFI；依赖图中**没有** `windows` 包 | **不引入，只观察** | FFI 面积继续增长、出现可归因于手写 FFI 的内存安全缺陷、需要成批新系统 API，或 Phase 5 AccessKit 把 `windows` 带入构建 |
| `image` | PNG 走 `png` 0.18，非 PNG 走系统 WIC；`image` 0.25.10 已经由 `arboard` 传入，但只启用 `bmp` + `png` feature | **不引入新 feature，只观察** | 产品决定支持 PNG 以外格式且不依赖系统编解码器、用户明确要求 WebP/GIF/AVIF 等、或 WIC 行为缺陷无法在参数层修复 |

两个组件的收益都真实，但当前**没有用户可感知的缺口**；体积余量只有约 498 KiB，任何替换都必须先按 Task 9 的「先测后换」称重。因此本阶段只记录触发条件，不动生产。

## 2. 现状盘点

### 2.1 手写 Win32 / COM FFI 面积

统计口径：生产代码中的 `extern "system"` 函数声明（`rg` 计数 79 条）与手写 COM vtable 槽位契约（16 个）；行数含测试与业务逻辑，不是纯 FFI 行数。

| 文件 | 行数 | extern 声明 | COM 槽位 | 职责 | 主要风险点 |
|---|---:|---:|---:|---|---|
| `crates/veil/src/platform/windows.rs` | 463 | 7 | 9 | 打开/保存对话框（`GetOpenFileNameW` + COM `IFileSaveDialog`） | vtable 下标、`OPENFILENAMEW` 布局、COM 生命周期 |
| `crates/veil/src/platform/wic.rs` | 428 | 3 | 7 | 非 PNG 图片解码（WIC → RGBA8） | vtable 下标、`RPC_E_CHANGED_MODE` 与 winit OLE 拖放共存 |
| `crates/veil/src/platform/print.rs` | 1,007 | 25 | 0 | GDI 打印、`PRINTDLGW`、DIB 贴帧 | 结构体布局、GDI 对象释放、DPI/分页 |
| `crates/veil/src/platform/assoc.rs` | 656 | 9 | 0 | 文件关联注册表 + `SHChangeNotify` | 注册表权限/视图、空值写入 `REG_NONE` |
| `crates/veil/src/platform/http.rs` | 1,228 | 12 | 0 | WinHTTP 远程图片抓取 | 句柄释放、错误码映射、超时 |
| `crates/veil/src/platform/ipc.rs` | 498 | 9 | 0 | 单实例命名管道 | 句柄所有权、会话边界 |
| `crates/veil/src/platform/theme.rs` | 250 | 5 | 0 | 注册表读主题、DWM 暗色、类背景 | 注册表缺省值/类型、窗口句柄有效性 |
| `crates/veil/src/platform/locale.rs` | 151 | 1 | 0 | 系统 UI 语言 | 低 |
| `crates/veil/src/platform/clock.rs` | 132 | 3 | 0 | 本地时间（状态栏「已保存」） | 时区结构布局 |
| `crates/veil/src/platform/error.rs` | 218 | 1 | 0 | `MessageBoxW` 兜底错误框 | 低 |
| `crates/veil/src/platform/shell.rs` | 79 | 1 | 0 | `ShellExecuteW` 打开 URL | 返回值 <= 32 的错误区间 |
| `crates/veil-ui/src/app.rs` | — | 1 | 0 | `SystemParametersInfoW` 取工作区 | 低 |
| `crates/veil-ui/src/perf.rs` | — | 2 | 0 | PSAPI 内存采样 | 低 |

另有 `crates/veil-ui/src/shell.rs` 的 8 个窗口枚举声明，位于 `#[cfg(test)]` 测试模块，不计入生产 FFI。

关键事实：

- **16 个 COM 槽位是最高风险面**：`windows.rs` 9 个（`IFileSaveDialog`/`IShellItem`），`wic.rs` 7 个（WIC 工厂/解码器/转换器）。槽位号写错仍能编译，运行期才崩溃。
- 手写 `#[repr(C)]` 结构覆盖 `OPENFILENAMEW`、`PRINTDLGW`、`DOCINFOW`、`BITMAPINFO`、`PROCESS_MEMORY_COUNTERS` 等，字段顺序/对齐由人工保证。
- 现有测试对部分危险面有覆盖（如 `wide()` 的 NUL 语义、WIC 解码成功/失败路径），但没有对 vtable 槽位与 ABI 布局的自动化校验。

### 2.2 `windows` / `windows-registry` 依赖现状

- `crates/veil/Cargo.toml`、`crates/veil-ui/Cargo.toml`、`Cargo.lock` 中**没有** `windows`、`windows-core`、`windows-registry` 包。
- 图中已有 `windows-sys`（0.52 / 0.59 / 0.61.2）。生产图里的 0.61.2 由 `arboard`、`notify`、`softbuffer`、`winapi-util` 传递引入；它是无类型的原始绑定，不能直接提供 COM 类型安全。
- Phase 5 的 AccessKit spike 已解析出 `windows 0.62.2` 及 21 个 `windows-*` 包（spike 解析节点 20 → 42）。如果生产接入并启用 `accessibility` feature，`windows` 会进入依赖图，届时迁移的边际包成本会显著下降。证据：`.trae/specs/remediate-product-trust-and-accessibility/accesskit-spike/RESULTS.md`。

### 2.3 图片解码现状（对任务背景的一处订正）

背景描述中的「仅 PNG 解码」只对**纯 Rust 路径**成立。当前实际分流在 `crates/veil/src/lib.rs:429`：

- PNG（魔数 `\x89PNG\r\n\x1a\n`）→ `decode_png_rgba`（`png` 0.18.1，`EXPAND | STRIP_16`）。
- 其余格式 → `platform::decode_image` → Windows WIC；JPEG/PNG/BMP/GIF/TIFF 由系统内建编解码器覆盖，WebP 依赖机器上已安装的 codec；只取第 0 帧，上限 1 亿像素（`wic.rs:56`）。
- `image` 0.25.10 已在生产依赖图（`arboard 3.7.0` 传递），但当前只启用 `bmp` + `png` feature；`jpeg`/`gif`/`webp`/`tiff` 解码器没有编译进产物。
- 直接依赖 `png` 0.18.1 与 `image` 传递的 `png` 0.17.16 并存。

因此 `image` 的收益不是「从 1 种格式到 N 种」，而是**把已支持的 N 种从系统 codec 换成进程内、跨机器确定的解码**。

## 3. `windows` / `windows-registry`

### 3.1 收益

| 维度 | 现状 | 引入后 |
|---|---|---|
| COM 类型安全 | 手写 vtable 下标 + `transmute_copy`（16 处） | 生成代码提供 typed 接口，槽位/签名由 crate 保证 |
| 结构体 ABI | 手写 `#[repr(C)]` | 生成绑定，字段顺序与对齐来自 Windows 元数据 |
| 句柄/BOOL/HRESULT | `*mut c_void`、`i32` | 类型化句柄与 `Result`，误用可在编译期发现 |
| 注册表 | 手写 `Reg*` 声明与 `KEY_ALL_ACCESS` | `windows-registry` 的 `Key` 抽象、值类型、递归删除 |
| 新增 API 成本 | 每次手抄声明与常量 | 加 feature 即可，维护性更好 |

### 3.2 体积 / 编译时间风险

- **体积未实测**（本任务禁止改 `Cargo.toml`）。`windows` 按命名空间 feature 门控，实际增量取决于启用范围；`windows-registry` 很薄。以下均为**推断**，采用前必须独立 spike 称重。
- 参照 AccessKit spike：`windows 0.62.2` 方案整体新增 22 个解析包、独立 spike 体积 +198 KiB（该数字含 accesskit 自身，不能直接归因给 `windows`）。
- `windows` 0.62 被拆分为 `windows-core` / `windows-strings` / `windows-result` / `windows-futures` 等多包，即使少量 feature 也会增加冷构建时间与 `target` 目录体积。
- **体积余量只有 509,952 B**：任何超过约 0.5 MB 的净增量都会直接触线，因此不能「先接上再测」。

### 3.3 行为差异

| 维度 | 现状 | 迁移后 | 影响 |
|---|---|---|---|
| 错误文本 | i18n `Key` + 十六进制 HRESULT（`ErrHresult`） | `windows::core::Error` 的 Display | 诊断文案变化；必须做映射，保留现有 i18n 归因 |
| 字符串 | `Vec<u16>` + 手动 NUL（`wide()`） | `HSTRING` / `PCWSTR` / `PWSTR` | 空串、内嵌 NUL 语义需重新验证（现有 4 个 `wide()` 单测） |
| COM 初始化 | `CoInitializeEx` + `RPC_E_CHANGED_MODE` 特判（`wic.rs:135`） | `windows-rs` 同样返回错误 | 不能简单 `?`，否则破坏与 winit OLE 拖放的共存 |
| 注册表 | `KEY_ALL_ACCESS`、`RegSetValueExW(REG_NONE)` | `windows-registry` 的值模型 | 权限最小化、32/64 位视图、空值语义需逐项确认 |
| 对话框 | `GetOpenFileNameW` 与 COM 槽位混用 | typed `IFileSaveDialog` 方法 | 参数顺序由生成代码保证，但 `SetFolder` 的确定性定位行为需真机复验 |

### 3.4 迁移成本

- 高价值/高风险优先：`windows.rs`（9 槽位）、`wic.rs`（7 槽位）。
- 面大量广：`print.rs`（25 个 API、多个 GDI 结构体）、`assoc.rs` / `theme.rs`（注册表语义）。
- 可后置：`http.rs`、`ipc.rs`、`clock.rs`、`locale.rs`、`error.rs`、`shell.rs`。
- 不是机械替换：需要重测全部错误路径、真机对话框/打印/文件关联行为，并对现有测试做等价性验证。按 Task 10「只观察」的定位，当前投入产出比不成立。

### 3.5 触发条件（任一满足即重开评估）

1. **FFI 面积继续增长**：生产 `extern` 入口超过约 100 个，或新增第 3 个手写 COM 接口族（当前 79 个 / 2 族：`IFileSaveDialog`+`IShellItem`、WIC）。
2. **内存安全缺陷**：出现可复现的崩溃/UB，归因于 vtable 槽位、`repr(C)` 布局或指针生命周期。
3. **成批新 API**：需要 UIA、DWM 扩展、Shell 扩展、打印高级特性等，手写声明持续增长（例如单季度超过约 300 行）。
4. **依赖图变化**：Phase 5 AccessKit 落地并把 `windows 0.62` 带入默认构建或常用 feature，边际包成本转为接近 0。
5. **ABI 证据需求**：需要跨 Windows 版本/架构的布局保证，手写代码无法提供可审计证据。

补充中间路径（同样不在本阶段采用）：直接依赖已在图中的 `windows-sys` 0.61 可以消除手写声明与部分布局风险，但不提供 COM 类型安全，`unsafe` 面积基本不变；若只为减少手抄声明，它比 `windows` 成本低，但仍需先称重。

## 4. `image`

### 4.1 收益

- **确定性**：同一个 `veil.exe` 在任意 Windows 机器上解码结果一致；WIC 依赖系统 codec 与 Windows 版本，WebP 尤其不稳定。
- **格式覆盖**：JPEG / GIF / WebP / TIFF 的纯 Rust 解码，不依赖机器已安装的编解码器。
- **路径统一**：PNG 与非 PNG 可共用一条解码路径，消除当前 `lib.rs` 的魔数分流。
- **供应链已存在**：`image` 0.25.10 已经由 `arboard` 带入，新增的是 feature 与解码器代码，不是新的依赖根节点。

### 4.2 风险

- **体积**：启用 `jpeg` / `gif` / `webp` / `tiff` 会编译 `zune-jpeg`、`gif`、`tiff`、`image-webp` 等（它们已在 `Cargo.lock`，但当前未编译）。未实测，**推断**为数百 KiB 量级，足以触及 14 MiB 门禁，必须先 spike 称重。
- **解码面 / 安全面**：不可信图片的解析面扩大；需要尺寸上限、帧数/动画策略、畸形样本不 panic 的回归。WIC 由系统更新兜底，进程内解码则由 Veil 自己承担。
- **行为差异**：EXIF 方向（两者默认都不自动应用，但需对拍确认）、ICC/色彩管理、alpha 预乘、GIF 动画（当前 WIC 只取首帧）、16-bit/调色板 PNG 处理。切换解码器可能改变用户看到的像素。
- **依赖卫生**：会同时存在 `png` 0.17（image）与 0.18（直接），除非一并收敛，否则重复版本增加审计面。

### 4.3 触发条件

1. **产品决定**支持 PNG 以外格式，且要求不依赖系统编解码器（离线/确定性发行契约）。
2. **用户明确需求**：WebP/AVIF/HEIC 在没有系统 codec 的机器上打不开，或需要 GIF 动画。
3. **WIC 行为缺陷**：EXIF 方向错误、颜色偏差、alpha 异常，且无法通过 WIC 参数层修复。
4. **导出格式扩展**：需要 PNG 之外的图片产物（当前仅公式/PDF 内部使用 PNG）。
5. 采用前必须完成 spike：实测 exe 增量、与 WIC 的逐像素对拍、畸形样本不 panic、许可证与 SBOM 更新。

## 5. 不引入的理由与再评估方式

- **理由**：当前没有用户可感知的缺口；体积余量 509,952 B；手写 FFI 有既有测试与可追溯的缺陷修复记录；Task 10 的验收只要求「有明确何时再评估」。
- **再评估入口**：任一触发条件满足 → 新开任务，按 Task 9 的六道门（质量、体积 ≤ 14 MiB、许可证、离线、无 webview、行为契约）做独立 spike，不直接进生产。
- **复核节奏**：Phase 5 AccessKit 落地后、体积门禁变更时、每次新增 Win32 子系统时重读本文。

## 6. 证据与复现

| 事实 | 命令/来源 |
|---|---|
| 生产 `extern "system"` 声明 79 条（另 8 条为测试） | `rg -n '^\s*(pub )?(unsafe )?fn [A-Z][A-Za-z0-9_]*\(' crates`（2026-11-26 共 87 行，扣除 `veil-ui/src/shell.rs` 测试模块 8 条） |
| `image` 仅由 `arboard` 传入，feature = `bmp` + `png` | `cargo tree --offline -e no-dev -p veil -i image@0.25.10`；`cargo tree --offline -e features -p veil -i image@0.25.10` |
| `windows` 包不存在；`windows-sys` 由 arboard/notify/softbuffer/winapi-util 传入 | `Cargo.lock` 检索；`cargo tree --offline -e no-dev -p veil -i windows-sys@0.61.2` |
| 体积 14,170,112 B / 门禁 14 MiB / 余量 509,952 B | `docs/review/evidence/perf/current-head-2026-11-26.md` |
| AccessKit spike 引入 `windows 0.62.2` 与 22 个新包 | `.trae/specs/remediate-product-trust-and-accessibility/accesskit-spike/RESULTS.md` |
| WIC 解码上限 1 亿像素 | `crates/veil/src/platform/wic.rs:56` |
| PNG / 非 PNG 分流 | `crates/veil/src/lib.rs:429-439` |

标记说明：本文只有「体积 14,170,112 B」「79/87 条声明」「`image` 的 feature 集」「`windows` 不在图中」是**实测/静态可验证**；迁移后的体积、编译时间增量均为**推断**，未在本阶段测量。
