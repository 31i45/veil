# 多语料基准（阶段 2 / P2 补充）
# 用法: powershell -File tools/bench/measure-suite.ps1 [-Trials 3]
# 对每份语料分别测：冷启动（窗口句柄）、进程树内存；逐应用逐语料一行汇总。
param(
    [int]$Trials = 3,
    [int]$SettleMs = 2000,
    [string]$OutRoot = "bench/results"
)

$root = Split-Path $PSScriptRoot -Parent | Split-Path -Parent
$corpusDir = Join-Path $root 'bench/corpus'
$suites = @(
    @{ Label = 'large-zh';  File = Join-Path $corpusDir 'large.md' },
    @{ Label = 'large-en';  File = Join-Path $corpusDir 'large-en.md' },
    @{ Label = 'patho-10mb'; File = Join-Path $corpusDir 'pathological-10mb.md' }
)
$apps = @(
    @{ Name = 'veil';     Exe = Join-Path $root 'target/release/veil.exe' },
    @{ Name = 'marktext'; Exe = 'C:\Program Files\marktext\marktext.exe' }
)

$ts = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
$outDir = Join-Path $root "$OutRoot/$ts-suite"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$rows = @()

foreach ($s in $suites) {
    if (-not (Test-Path $s.File)) {
        Write-Host ("跳过 {0}：缺少 {1}（先运行 python tools/bench/gen_corpus.py --out bench/corpus）" -f $s.Label, $s.File)
        continue
    }
    foreach ($a in $apps) {
        if (-not (Test-Path $a.Exe)) { continue }
        $dir = Split-Path $a.Exe -Parent
        $walls = @(); $mems = @(); $resp = @()
        for ($i = 1; $i -le $Trials; $i++) {
            Get-Process -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and $_.Path.StartsWith($dir) } |
                Stop-Process -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 300
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $p = Start-Process -FilePath $a.Exe -ArgumentList $s.File -PassThru
            while (-not $p.MainWindowHandle -and $sw.ElapsedMilliseconds -lt 15000) {
                Start-Sleep -Milliseconds 10; $p.Refresh()
            }
            $walls += $sw.ElapsedMilliseconds
            Start-Sleep -Milliseconds $SettleMs
            $tree = Get-Process -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and $_.Path.StartsWith($dir) }
            $mems += [math]::Round((($tree | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 2)
            $resp += ($tree | Where-Object { $_.MainWindowHandle -ne 0 }).Responding -join '/'
            $tree | Stop-Process -Force -ErrorAction SilentlyContinue
        }
        $w = $walls | Sort-Object; $m = $mems | Sort-Object
        $row = [pscustomobject]@{
            suite = $s.Label; app = $a.Name
            cold_median_ms = $w[[int]($Trials/2)]
            cold_min_ms = $w[0]; cold_max_ms = $w[-1]
            mem_median_mb = $m[[int]($Trials/2)]
            responding = ($resp -join ',')
        }
        $rows += $row
        Write-Host ("{0,-11} {1,-9} cold={2}ms mem={3}MB resp={4}" -f $row.suite, $row.app, $row.cold_median_ms, $row.mem_median_mb, $row.responding)
    }
}
$rows | Export-Csv (Join-Path $outDir 'suite.csv') -NoTypeInformation -Encoding utf8
Write-Host "`n写入 $outDir"
