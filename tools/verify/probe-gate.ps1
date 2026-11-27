# Veil ignored 探针门禁（薄运行器）
#
# 唯一职责：release 构建 → 一次性跑完全部 #[ignore] 探针 → 归档输出 → 透传 cargo 退出码。
# 判定全部下沉到探针自身：每条探针在测试内断言回归上限（regression ceiling）或结构不变量。
# 本脚本不再保存基线 JSON、不解析指标正则、不设 p50/p95 阈值或噪声地板，也不逐条 filter 运行。
#
# 串行执行（RUST_TEST_THREADS=1）：性能探针在同一进程内并行会互相抢核，
# 使探针内自证上限失真，因此固定单线程。
#
# 用法：pwsh -NoProfile -File tools/verify/probe-gate.ps1

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$dateStamp = Get-Date -Format 'yyyy-MM-dd'
$timestamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$archivePath = Join-Path $repoRoot "docs\qa\probe-gate-$dateStamp.md"
$cargo = (Get-Command cargo -ErrorAction Stop).Source

# 性能探针必须串行，避免互相干扰导致探针内自证上限假红。
$env:RUST_TEST_THREADS = '1'

$buildArgs = @('test', '--workspace', '--release', '--no-run')
$runArgs = @('test', '--workspace', '--release', '--', '--ignored', '--nocapture')

function Invoke-CargoCaptured {
    param([string[]]$Arguments)
    $lines = [System.Collections.Generic.List[string]]::new()
    & $cargo @Arguments 2>&1 | ForEach-Object {
        $line = $_.ToString()
        Write-Host $line
        $lines.Add($line)
    }
    return [pscustomobject]@{ Lines = $lines; Code = $LASTEXITCODE }
}

function Get-Tail {
    param([System.Collections.Generic.List[string]]$Lines, [int]$Count)
    if ($Lines.Count -le $Count) { return ($Lines -join "`n") }
    return (($Lines[($Lines.Count - $Count)..($Lines.Count - 1)]) -join "`n")
}

function Write-Archive {
    param([string]$Body)
    $dir = Split-Path -Parent $archivePath
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    [IO.File]::WriteAllText($archivePath, $Body, [Text.UTF8Encoding]::new($false))
    Write-Host "结果归档：$archivePath"
}

$machine = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$os = Get-CimInstance Win32_OperatingSystem
$osText = "$($os.Caption) $($os.Version)"
$rustcVersion = (& rustc -V) -join ''

# ---------- 1/2 release 测试二进制构建 ----------
Write-Host "=== 1/2 构建：cargo $($buildArgs -join ' ') ==="
$buildSw = [System.Diagnostics.Stopwatch]::StartNew()
$build = Invoke-CargoCaptured -Arguments $buildArgs
$buildSw.Stop()
$buildSec = [math]::Round($buildSw.Elapsed.TotalSeconds, 1)

if ($build.Code -ne 0) {
    Write-Host "构建失败（exit $($build.Code)）"
    $md = @"
# Probe gate 结果（$dateStamp）

> 本文件由 ``tools/verify/probe-gate.ps1`` 自动生成，重复运行整体覆盖。

- 时间：$timestamp
- 机器：$machine / $osText
- 工具链：$rustcVersion
- 结果：**FAIL（release 构建失败，exit $($build.Code)）**
- 构建耗时：${buildSec} s

## 命令

``````text
cargo test --workspace --release --no-run
cargo test --workspace --release -- --ignored --nocapture
``````

## 构建输出尾部

``````text
$(Get-Tail -Lines $build.Lines -Count 120)
``````
"@
    Write-Archive -Body $md
    exit $build.Code
}

Write-Host "构建完成：${buildSec}s"

# ---------- 2/2 一次性跑完全部 ignored 探针 ----------
Write-Host "=== 2/2 运行：cargo $($runArgs -join ' ') ==="
$runSw = [System.Diagnostics.Stopwatch]::StartNew()
$run = Invoke-CargoCaptured -Arguments $runArgs
$runSw.Stop()
$runSec = [math]::Round($runSw.Elapsed.TotalSeconds, 1)

$resultLines = @($run.Lines | Where-Object { $_ -match '^test result: ' })
$summary = if ($resultLines.Count -gt 0) {
    ($resultLines | ForEach-Object { "- ``$_``" }) -join "`n"
} else {
    '- （输出中没有 `test result:` 行）'
}
$failedTotal = 0
foreach ($line in $resultLines) {
    if ($line -match '(\d+) failed') { $failedTotal += [int]$Matches[1] }
}
$verdict = if ($run.Code -eq 0) { '**PASS**（cargo exit 0）' } else { "**FAIL**（cargo exit $($run.Code)）" }

$md = @"
# Probe gate 结果（$dateStamp）

> 本文件由 ``tools/verify/probe-gate.ps1`` 自动生成，重复运行整体覆盖。
> 判定在探针内部完成（测试内断言回归上限/结构不变量）；本脚本只运行与归档。

- 时间：$timestamp
- 机器：$machine / $osText
- 工具链：$rustcVersion
- 结果：$verdict
- 构建耗时：${buildSec} s
- 运行耗时：${runSec} s
- 串行执行：``RUST_TEST_THREADS=1``（避免性能探针互相抢 CPU）
- 失败用例总数（解析 ``test result:`` 行）：$failedTotal

## 命令

``````text
cargo test --workspace --release --no-run
cargo test --workspace --release -- --ignored --nocapture
``````

## 探针结果行

$summary

## 输出尾部（最后 120 行）

``````text
$(Get-Tail -Lines $run.Lines -Count 120)
``````
"@

Write-Archive -Body $md
Write-Host "PASS/FAIL：$verdict（构建 ${buildSec}s / 运行 ${runSec}s）"
exit $run.Code
