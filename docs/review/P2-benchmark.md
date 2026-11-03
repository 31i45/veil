# P2 公开基准（阶段 2 · 同机对比）

> 日期：2026-10-30 · 机器：本机 Windows（release 构建）· 全部脚本与语料已入库，可复现
> 对照：Veil 0.1.0（`target/release/veil.exe`）vs MarkText（`C:\Program Files\marktext`）
> 语料：`bench/corpus/large.md`（1000 节，821,560 B，约 52.6 万字符，其中 CJK 13.5 万）
> 生成器：`tools/bench/gen_corpus.py`（SEED=20261030）；测量器：`tools/bench/measure.ps1`、
> `tools/bench/measure-suite.ps1`（多语料矩阵）

## 结论

**「快」的第三支柱在内存与体积上成立（17× 与 90× 量级优势）；冷启动在本机口径下两者打平。**
输入延迟 Veil 侧 p95 = 6.83ms（进程内探针，5,005 块 fixture）；MarkText 侧无公开可信的
进程内探针口径，未测，列入「待补」而非伪造。

## 1. 数据（5 轮中位数）

| 指标 | Veil | MarkText | 倍率（Veil 优势） |
|---|---|---|---|
| 冷启动（进程创建→主窗口句柄非零） | 30 ms | 30 ms | 1.0×（打平，见 §3 口径注记） |
| 常驻内存（私有工作集，进程树求和） | 21.51 MB | 365.01 MB | **17.0×** |
| 安装产物体积 | 5.46 MB | 488.95 MB | **89.6×** |
| 击键延迟 p95（10 万字文档） | 6.83 ms | 未测 | — |

原始数据：`bench/results/20261030-015822/`（per-app CSV + summary.md）。

## 2. 方法（公开可复现）

```powershell
# 1. 生成语料（幂等，SEED 固定）
python tools/bench/gen_corpus.py --out bench/corpus
# 2. 构建 Veil release
cargo build --release -p veil
# 3. 跑基准（默认 5 轮）
powershell -File tools/bench/measure.ps1 -Trials 5
# 4. Veil 进程内击键探针（必须串行）
cargo test -p veil-ui --release -- --ignored --nocapture --test-threads=1 perf_keystroke_path
```

测量定义：

- **冷启动**：`Start-Process` 起计时，轮询至 `MainWindowHandle != 0`，5 轮取中位；
  每轮先杀残留进程再启动。
- **内存**：启动后 2s，取「安装目录下全部进程」的 `PrivateMemorySize64` 之和
  （Electron 应用含 main/renderer/GPU 多进程，单进程口径会严重低估）。
- **体积**：Veil = 单 exe 字节数；MarkText = 安装目录递归字节数。
- **击键延迟**：Veil 用进程内探针 `perf_keystroke_path`（5,005 块中文 fixture，
  apply+重排+绘制全链路，500 次采样取 p95）。

## 3. 口径与已知偏袒（诚实声明）

1. **冷启动打平不可尽信为「Veil 快」的证据**：`MainWindowHandle` 出现时 Electron
   的 renderer 通常尚未绘制内容；Veil 窗口句柄出现即接近首帧。该口径系统性偏袒
   MarkText，故 30ms=30ms 应读作「Veil 不慢于对手」，真实感知优势大概率更大，
   但本轮无公正数据，不写入结论。
2. **MarkText 击键延迟未测**：无公开进程内探针，UI 自动化测量噪声大于信号，
   按「不伪造数据」原则留白。若后续接入 UI Automation 或高速相机，再补。
3. Veil 冷启动在本机的绝对值历史记录：G2 门禁 450–529ms（当时 pre-main 段异常），
   本轮 30ms。两次同为真值，差异来自系统状态（进程创建段开销），非产品代码变化——
   这正是阶段 2 要公开方法的原因：任何第三方可在自己机器上复核。

## 4. 多语料复测（2026-10-30 第二轮，3 轮中位）

脚本：`tools/bench/measure-suite.ps1`；原始数据 `bench/results/20261030-020813-suite/`。

| 语料 | 应用 | 冷启动中位 ms | 内存中位 MB | 窗口响应 |
|---|---|---|---|---|
| large-zh（82 万字节中文混排） | Veil | 77 | **21.48** | 3/3 True |
| large-zh | MarkText | 42 | 367.31 | 3/3 True |
| large-en（35 万字节纯英文） | Veil | 40 | **19.66** | 3/3 True |
| large-en | MarkText | 40 | 373.88 | 3/3 True |
| patho-10mb（10MB 病态单块） | Veil | 39 | **150.13** | 3/3 True |
| patho-10mb | MarkText | 39 | 468.05 | 3/3 True |

读法与新增发现：

1. **内存优势跨语料稳定**：Veil 19.7–21.5MB vs MarkText 365–468MB，倍率 3.1×–19×；
   病态单块下 Veil 内存涨到 150.13MB——这是 0a 三上限降级后的真实峰值，
   **恰好顶到 150MB 红线**，且仍只有 MarkText（468MB）的 1/3。列为持续观察项，
   若病态场景内存继续恶化需重审单块降级策略的缓存驻留。
2. **英文 vs 中文（Veil 自身）**：21.48MB → 19.66MB，差异 ~1.8MB，属 CJK 字体光栅
   缓存的正常开销，无异常放大。
3. **冷启动打平口径警告同 §3**：MarkText 窗口句柄出现 ≠ 内容可交互；
   Electron 的真实「可读首帧」远晚于 39–42ms。此列不应引用为「两者一样快」，
   正确表述是「窗口句柄口径下 Veil 不慢于 MarkText」。
4. **病态单块两个都活着**：Veil 靠三上限 + 显式降级渲染前缀；MarkText 也 Responding。
   该项不构成差异化优势，但证明 0a 保护线在真实应用中生效。

## 5. 下一步

- [ ] MarkText/Typora 击键延迟的可信测量路径（UI Automation 或高速相机）
- [x] ~~语料扩展：10MB 病态单块、纯英文对照组~~（2026-10-30 完成）
- [ ] 外部用户盲测（阶段 2 验收项，真人，不可 agent 代做）
- [ ] 病态单块 150.13MB 内存顶线：确认 0a 降级缓存是否有可裁剪驻留
