# Veil 实体打印机 assisted 取证（Task 5）
#
# 用户只做一件事：装纸 → 在应用里 Ctrl+Alt+P 选择实体打印机 → 打印 → 按清单核对出纸。
# 脚本负责：生成含「短代码块 + 跨页长表格 + 图片」的样张、列出系统打印机/driver、
# 生成核对清单（每页表头、行边界、页码、页数）、打开样张并给出操作提示。
# 无法自动确认的出纸项一律写 PENDING；没有实体打印机时写 SKIP，绝不写 PASS。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/assisted/print.ps1            # 生成样张/清单并打开应用
#   pwsh -NoProfile -File tools/verify/assisted/print.ps1 -NoLaunch  # 只生成样张与清单（CI/自动化）
#
# 退出码：0 = 清单已生成（含 SKIP/PENDING）；1 = 执行失败。

[CmdletBinding()]
param(
    [switch]$NoLaunch,
    [string]$Exe = '',
    [string]$OutDir = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$dateStamp = '2026-11-27'
if (-not $OutDir) { $OutDir = Join-Path $repoRoot "docs\qa\evidence\assisted-$dateStamp" }
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

$samplePath = Join-Path $OutDir 'print-sample.md'
$checklistPath = Join-Path $OutDir 'print-checklist.md'
$imageName = 'print-sample-image.png'
$imageTarget = Join-Path $OutDir $imageName

# ---------- 1. 样张图片（复用仓库内既有证据图；找不到就明确 SKIP，不伪造）----------
$imageSource = Get-ChildItem -LiteralPath (Join-Path $repoRoot 'docs\qa\evidence') -Recurse -Filter '*.png' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notlike "*assisted-$dateStamp*" } |
    Sort-Object Length -Descending | Select-Object -First 1
if ($null -ne $imageSource) {
    Copy-Item -LiteralPath $imageSource.FullName -Destination $imageTarget -Force
    $imageStatus = "OK（来源 $($imageSource.FullName)）"
    $imageLine = "![Veil 样张图片]($imageName)"
} else {
    $imageStatus = 'SKIP（未找到可复用的既有 png，未伪造图片）'
    $imageLine = '> [SKIP] 本轮未找到仓库内既有 png，样张不含图片。'
}

# ---------- 2. 样张：短代码块 + 跨页长表格 + 图片 ----------
$sb = [System.Text.StringBuilder]::new()
[void]$sb.AppendLine('# Veil 实体打印样张（A4）')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('> 用途：核对短代码块不跨页、长表格每页有表头、断页落在行边界、页码连续。')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('## 1. 短代码块（应完整落在同一页）')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('```rust')
[void]$sb.AppendLine('fn print_probe() -> u32 {')
[void]$sb.AppendLine('    42')
[void]$sb.AppendLine('}')
[void]$sb.AppendLine('```')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('## 2. 图片（应完整、不裁切）')
[void]$sb.AppendLine('')
[void]$sb.AppendLine($imageLine)
[void]$sb.AppendLine('')
[void]$sb.AppendLine('## 3. 超过一页的长表格（跨页时每页须重复表头）')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('| 行号 | 名称 | 说明 | 数值 |')
[void]$sb.AppendLine('| --- | --- | --- | --- |')
for ($i = 1; $i -le 80; $i++) {
    [void]$sb.AppendLine("| $i | 第 $i 行 | 用于验证跨页断行与表头重复的中文长文本条目 $i | $($i * 17) |")
}
[void]$sb.AppendLine('')
[void]$sb.AppendLine('## 4. 尾部段落')
[void]$sb.AppendLine('')
[void]$sb.AppendLine('打印核对完成后，请在 `print-checklist.md` 中把每个 PENDING 改成 PASS/FAIL 并附照片路径。')
[IO.File]::WriteAllText($samplePath, $sb.ToString(), [Text.UTF8Encoding]::new($false))

# ---------- 3. 系统打印机 ----------
$printers = @()
try { $printers = @(Get-Printer -ErrorAction Stop) } catch { $printers = @() }
$physical = @($printers | Where-Object {
        $_.Name -notmatch 'Microsoft Print to PDF|Microsoft XPS|OneNote|Fax' -and $_.Name -notmatch '^\s*$'
    })

function Format-PrinterRows {
    param($Items)
    if ($Items.Count -eq 0) { return '| （无） | — | — | — |' }
    return (($Items | ForEach-Object { "| $($_.Name) | $($_.DriverName) | $($_.PortName) | $($_.PrinterStatus) |" }) -join "`n")
}

$physicalRows = Format-PrinterRows $physical
$allRows = Format-PrinterRows $printers
$printerStatus = if ($physical.Count -gt 0) {
    "可用实体打印机 $($physical.Count) 台"
} else {
    'SKIP（未检测到实体打印机，仅 Microsoft Print to PDF 等虚拟打印机）'
}

$stamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$machine = $env:COMPUTERNAME
$os = 'Windows'
try { $os = (Get-CimInstance Win32_OperatingSystem).Caption } catch { }

$checklist = @"
# 实体打印机核对清单（$dateStamp）

> 本文件由 ``tools/verify/assisted/print.ps1`` 自动生成，出纸项无法自动确认，默认 PENDING。
> 纪律：没有真实出纸核对就保持 PENDING，不得改成 PASS；没有实体打印机时整节记 SKIP。

- 时间：$stamp
- 机器：$machine / $os
- 样张：``$samplePath``
- 图片：$imageStatus
- 打印机检测：$printerStatus

## 系统打印机（Get-Printer）

| 名称 | 驱动 | 端口 | 状态 |
|---|---|---|---|
$allRows

### 实体打印机候选

| 名称 | 驱动 | 端口 | 状态 |
|---|---|---|---|
$physicalRows

## 用户只需执行

1. 装好 A4 纸；双击打开 ``$samplePath``（或脚本已自动打开 veil.exe）。
2. 按 ``Ctrl+Alt+P`` → 选择上表中的**实体打印机** → 纸张 A4 → 打印。
3. 按下面清单逐项核对出纸，把 ``PENDING`` 改为 ``PASS`` / ``FAIL``，并登记照片路径。
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
- 驱动造成的字体度量差异按 ``docs/review/evidence/layout-consistency-2026-11-26.md``
  的「已接受的环境差异」处理，不作为缺陷。
- 本轮自动部分（样张/打印机枚举/清单）已完成；出纸项保持 **PENDING（未执行）**。
"@

[IO.File]::WriteAllText($checklistPath, $checklist, [Text.UTF8Encoding]::new($false))
Write-Host "样张：$samplePath"
Write-Host "核对清单：$checklistPath"
Write-Host "打印机：$printerStatus"
Write-Host '出纸核对项保持 PENDING：需要你装纸、Ctrl+Alt+P 选择实体打印机并核对出纸。'

if (-not $NoLaunch) {
    if (-not (Test-Path -LiteralPath $Exe)) {
        Write-Host "找不到 release 可执行文件：$Exe（可先 cargo build --release -p veil）；样张与清单已生成。" -ForegroundColor Yellow
    } else {
        $p = Start-Process -FilePath $Exe -ArgumentList @($samplePath) -PassThru
        Write-Host "已打开 veil.exe（PID=$($p.Id)）。请按 Ctrl+Alt+P 打印并核对出纸。"
    }
} else {
    Write-Host '（-NoLaunch：本次未启动应用。）'
}

exit 0
