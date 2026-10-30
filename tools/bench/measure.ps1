# Veil 公开基准测量脚本（阶段 2 / P2）
# 用法: powershell -File tools/bench/measure.ps1 [-Trials 5]
# 产出: bench/results/<UTC 时间戳>/ 下每应用 CSV + 汇总 summary.md
param(
    [int]$Trials = 5,
    [int]$SettleMs = 2000,
    [string]$OutRoot = "bench/results"
)

$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent | Split-Path -Parent   # repo root
$corpus = Join-Path $root 'bench/corpus/large.md'
if (-not (Test-Path $corpus)) { throw "语料缺失: $corpus，先跑 tools/bench/gen_corpus.py" }

# ── 被测应用注册表 ──────────────────────────────────────────────
# 每项: Name, Exe, Args(可带 {doc} 占位), SizePath(安装产物计量路径, 目录则递归)
$apps = @(
    @{
        Name = 'veil'
        Exe  = Join-Path $root 'target/release/veil.exe'
        Args = '{doc}'
        SizePath = Join-Path $root 'target/release/veil.exe'
    },
    @{
        Name = 'marktext'
        Exe  = 'C:\Program Files\marktext\marktext.exe'
        Args = '{doc}'
        SizePath = 'C:\Program Files\marktext'
    }
)

$ts = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
$outDir = Join-Path $root "$OutRoot/$ts"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function Get-SizeBytes($path) {
    if (Test-Path $path -PathType Leaf) { return (Get-Item $path).Length }
    (Get-ChildItem $path -Recurse -File -ErrorAction SilentlyContinue |
        Measure-Object Length -Sum).Sum
}

function Measure-App($app) {
    $name = $app.Name
    if (-not (Test-Path $app.Exe)) { Write-Warning "$name 未安装: $($app.Exe)"; return $null }
    $docArg = $app.Args -replace '\{doc\}', $corpus
    $rows = @()
    for ($i = 1; $i -le $Trials; $i++) {
        # 杀残留进程，避免「激活已开窗口」假性快启动
        Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($app.Exe)) -ErrorAction SilentlyContinue |
            Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 300

        $sw = [Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process -FilePath $app.Exe -ArgumentList $docArg -PassThru
        # 等主窗口出现（GUI 可交互代理）；8s 超时记为 fail
        while (-not $p.MainWindowHandle -and $sw.ElapsedMilliseconds -lt 8000) {
            Start-Sleep -Milliseconds 10; $p.Refresh()
        }
        $wallMs = $sw.ElapsedMilliseconds
        Start-Sleep -Milliseconds $SettleMs
        # 内存按「安装目录下全部同路径进程」求和：Electron 应用主进程+renderer+GPU 才算公平口径
        $dir = Split-Path $app.Exe -Parent
        $memBytes = (Get-Process -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path.StartsWith($dir) } |
            Measure-Object PrivateMemorySize64 -Sum).Sum
        if (-not $memBytes) { $p.Refresh(); $memBytes = $p.PrivateMemorySize64 }
        $memMb = [math]::Round($memBytes / 1MB, 2)
        # 清场：杀整棵树
        Get-Process -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path.StartsWith($dir) } |
            Stop-Process -Force -ErrorAction SilentlyContinue
        $rows += [pscustomobject]@{ trial = $i; wall_ms = $wallMs; mem_mb = $memMb }
    }
    $sizeBytes = Get-SizeBytes $app.SizePath
    $sizeMb = [math]::Round($sizeBytes / 1MB, 2)
    $wall = $rows.wall_ms | Sort-Object
    $med = $wall[[int]($Trials / 2)]
    [pscustomobject]@{
        app        = $name
        trials     = $Trials
        cold_median_ms = $med
        cold_min_ms    = $wall[0]
        cold_max_ms    = $wall[-1]
        mem_median_mb  = ($rows.mem_mb | Sort-Object)[[int]($Trials / 2)]
        size_mb        = $sizeMb
    }
}

$summary = @()
foreach ($a in $apps) {
    $r = Measure-App $a
    if ($r) {
        $summary += $r
        $r | ConvertTo-Csv -NoTypeInformation | Out-File (Join-Path $outDir "$($r.app).csv") -Encoding utf8
        Write-Host ("{0,-10} cold={1}ms (min {2}, max {3})  mem={4}MB  size={5}MB" -f `
            $r.app, $r.cold_median_ms, $r.cold_min_ms, $r.cold_max_ms, $r.mem_median_mb, $r.size_mb)
    }
}

# 汇总 markdown（可直接贴进报告）
$md = @("# P2 基线（$ts）", "", "| 应用 | 冷启动中位 ms | 最小 | 最大 | 内存 MB | 体积 MB |", "|---|---|---|---|---|---|")
foreach ($r in $summary) {
    $md += "| $($r.app) | $($r.cold_median_ms) | $($r.cold_min_ms) | $($r.cold_max_ms) | $($r.mem_median_mb) | $($r.size_mb) |"
}
$md -join "`n" | Out-File (Join-Path $outDir 'summary.md') -Encoding utf8
Write-Host "`n写入 $outDir"
