# G4 加载态实测（2026-11-27）

## 目的

裁决异常矩阵 G4：「大纲 / 快速打开 / 历史面板无加载态，超大目录会阻塞 UI」是否需要立项异步加载态。阈值：**快速打开或大纲的同步阻塞 > 100 ms 才立项**；否则关闭 G4 并写明依据。

## 环境

| 项 | 值 |
|---|---|
| 机器 | Intel Core i9-14900HX |
| 系统 | Windows（NTFS，D 盘） |
| 工具链 | rustc 1.99.1 / cargo 1.99.1 |
| 构建 | `--release`（另附 debug 对照） |
| 日期 | 2026-11-27 |

## 方法

在 `crates/veil-ui/src/view/quick_open.rs` 的测试模块临时加入一个 `#[ignore]` 测量用例（**测后已移除，未进入提交**）：

1. 在 `%TEMP%` 建 5000 个 `note-00000.md … note-04999.md`。
2. 按产品实际路径测量：
   - 目录枚举：`std::fs::read_dir` + `file_type().is_file()`（与 `FileStore::list_files` 同形）；
   - 候选构建：`quick_open::candidates(&[], &files, CANDIDATE_CAP)`；
   - 查询重排：`quick_open::filter(&items, "note-049")`（面板每次按键的重排口径）。
3. 冷启一次，预热 3 次，取 20 次样本，输出 min / median / p95 / max。
4. 大纲来源：对 5000 个标题的文档执行 `veil_core::Session::open`，再枚举 `AtxHeading` 块。

命令：

```powershell
cargo test  -p veil-ui tmp_w11_measure_5000_files -- --ignored --nocapture
cargo test --release -p veil-ui tmp_w11_measure_5000_files -- --ignored --nocapture
```

## 原始结果

### release（产品口径）

```text
[W11] cold list_dir: 11.498ms（5000 项）
[W11] list_dir(5000): n=20 min=10.266ms median=10.959ms p95=12.885ms max=12.885ms
[W11] candidates(5000→cap500): n=20 min=1.878ms median=1.931ms p95=2.044ms max=2.044ms
[W11] refilter(query, cap500): n=20 min=0.468ms median=0.485ms p95=0.562ms max=0.562ms
[W11] Session::open(5000 headings): 148.385ms
[W11] enumerate headings: 0.344ms（5000 个）
```

### debug（对照，非产品口径）

```text
[W11] cold list_dir: 14.767ms（5000 项）
[W11] list_dir(5000): n=20 min=12.076ms median=12.688ms p95=13.713ms max=13.713ms
[W11] candidates(5000→cap500): n=20 min=3.908ms median=3.996ms p95=4.380ms max=4.380ms
[W11] refilter(query, cap500): n=20 min=1.804ms median=1.867ms p95=2.277ms max=2.277ms
[W11] Session::open(5000 headings): 2556.362ms
[W11] enumerate headings: 0.780ms（5000 个）
```

## 结论

| 路径 | release 实测 | 与 100 ms 阈值 | 处置 |
|---|---|---|---|
| 快速打开（5000 文件，列目录 + 候选 + 重排） | 约 **13 ms** | 远低于 | **关闭 G4**，不引入异步加载态 |
| 大纲（5000 标题块枚举） | **0.34 ms** | 远低于 | **关闭 G4**，不引入异步加载态 |
| 版本历史面板 | 未单独实测（同目录枚举量级，无进度态） | 未验证 | 保留矩阵既有「可能短暂阻塞」的如实表述，不据此声称达标 |

**附注（如实记录，不作为 G4 关闭的依据）**：5000 标题文档的 `Session::open` 在 release 下约 **148 ms**，属于「文档打开/解析」路径，不是快速打开或大纲面板的缺口。本轮不引入加载态，也不将其表述为「大文档打开流畅」。

## 局限

- 单机、本地 NTFS、热缓存；未覆盖网络盘、杀毒软件实时扫描、低配机器。
- 「历史面板」未单独构造 5000 快照实测；其列表读取与快速打开同为同步目录枚举，风险量级相近但未取证。
- 测量代码为临时用例，已从工作区移除；本文件保留方法与原始输出，便于复现。
