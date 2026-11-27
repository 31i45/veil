# 实体打印机核对清单（2026-11-27）

> 本文件由 `tools/verify/assisted/print.ps1` 自动生成，出纸项无法自动确认，默认 PENDING。
> 纪律：没有真实出纸核对就保持 PENDING，不得改成 PASS；没有实体打印机时整节记 SKIP。

- 时间：2026-11-27 23:59:13 +08:00
- 机器：DESKTOP-K0DU5LK / Microsoft Windows 11 企业版
- 样张：`D:\VSCode\Veil\docs\qa\evidence\assisted-2026-11-27\print-sample.md`
- 图片：OK（来源 D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27\print-pdf-mupdf-p4.png）
- 打印机检测：可用实体打印机 1 台

## 系统打印机（Get-Printer）

| 名称 | 驱动 | 端口 | 状态 |
|---|---|---|---|
| Pantum CM230DN Series 0001 | Pantum CM230DN Series | IPP_192.168.17.250_1 | Normal |
| Microsoft Print to PDF | Microsoft Print To PDF | PORTPROMPT: | Normal |

### 实体打印机候选

| 名称 | 驱动 | 端口 | 状态 |
|---|---|---|---|
| Pantum CM230DN Series 0001 | Pantum CM230DN Series | IPP_192.168.17.250_1 | Normal |

## 用户只需执行

1. 装好 A4 纸；双击打开 `D:\VSCode\Veil\docs\qa\evidence\assisted-2026-11-27\print-sample.md`（或脚本已自动打开 veil.exe）。
2. 按 `Ctrl+Alt+P` → 选择上表中的**实体打印机** → 纸张 A4 → 打印。
3. 按下面清单逐项核对出纸，把 `PENDING` 改为 `PASS` / `FAIL`，并登记照片路径。
4. 若本机没有实体打印机，本节整体记 **SKIP**，不得写 PASS。

## 出纸核对项（自动无法确认 → PENDING）

| # | 核对项 | 判定标准 | 状态 | 证据/备注 |
|---|---|---|---|---|
| 1 | 短代码块 | 3 行代码块完整在同一页，未被上下切开 | PENDING | — |
| 2 | 长表格表头 | 每一页顶部都有「行号 / 名称 / 说明 / 数值」表头 | PENDING | — |
| 3 | 断页行边界 | 断页落在两行之间，没有某行被上下切成两半 | PENDING | — |
| 4 | 页码连续 | 页脚页码从 1 开始连续，无缺页/重页 | PENDING | — |
| 5 | 页数一致 | 总页数与「Microsoft Print to PDF」输出同一文档的页数一致 | PENDING | — |
| 6 | 图片完整 | 图片完整显示、不裁切、不跨页 | PENDING | — |

## 判定

- 六项全部 PASS 才算实体打印达标；任一项 FAIL 记录驱动名、纸型与照片路径，并登记缺陷。
- 驱动造成的字体度量差异按 `docs/review/evidence/layout-consistency-2026-11-26.md`
  的「已接受的环境差异」处理，不作为缺陷。
- 本轮自动部分（样张/打印机枚举/清单）已完成；出纸项保持 **PENDING（未执行）**。