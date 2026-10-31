# Veil 冷启动环境段（pre-main）诊断脚本（阶段 3 / 切片 3a）
# 用途：区分「进程创建段平台开销」与「应用内开销」。三根探针：
#   1. 未签名空二进制地板（%TEMP% 下 124KB hello-world，start→exit）
#   2. 微软签名二进制地板（system32 cmd.exe 副本放 %TEMP%，start→exit）
#   3. veil.exe 墙钟（start→主窗口句柄，口径同 measure.ps1）
# 解读：1/2 远高于 veil.exe 墙钟 ⇒ 环境（杀软 HIPS 信誉拦截），非产品代码问题。
# 用法: powershell -File tools/bench/diag-premain.ps1 [-Trials 10]
param([int]$Trials = 10)

$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent | Split-Path -Parent
$diag = Join-Path $env:TEMP 'veil-diag'
New-Item -ItemType Directory -Force -Path $diag | Out-Null

function Measure-Exit($exe, [string[]]$argList, $n) {
    $t = @()
    1..$n | ForEach-Object {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process $exe -ArgumentList $argList -PassThru
        $p.WaitForExit()
        $t += $sw.ElapsedMilliseconds
    }
    $s = $t | Sort-Object
    [pscustomobject]@{ median = $s[[int]($n / 2)]; min = $s[0]; max = $s[-1] }
}

# 探针 1：未签名空二进制
$helloSrc = Join-Path $diag 'hello.rs'
$helloExe = Join-Path $diag 'hello.exe'
Set-Content $helloSrc 'fn main() { std::process::exit(0); }'
rustc -O $helloSrc -o $helloExe
if ($LASTEXITCODE -ne 0) { throw "rustc 不可用" }
$hello = Measure-Exit $helloExe @() $Trials

# 探针 2：微软签名二进制（同目录、同测法）
$msft = Join-Path $diag 'msft-signed.exe'
Copy-Item "$env:SystemRoot\System32\cmd.exe" $msft -Force
$signed = Measure-Exit $msft @('/c', 'exit') $Trials

# 探针 3：veil.exe 墙钟（杀残留后 start→主窗口句柄）
$veilExe = Join-Path $root 'target/release/veil.exe'
$t = @()
1..$Trials | ForEach-Object {
    Get-Process -Name veil -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $veilExe -PassThru
    while (-not $p.MainWindowHandle -and $sw.ElapsedMilliseconds -lt 8000) {
        Start-Sleep -Milliseconds 10; $p.Refresh()
    }
    $t += $sw.ElapsedMilliseconds
    $p | Stop-Process -Force -ErrorAction SilentlyContinue
}
$s = $t | Sort-Object
$veil = [pscustomobject]@{ median = $s[[int]($Trials / 2)]; min = $s[0]; max = $s[-1] }

"未签名空二进制  start→exit  中位 $($hello.median) ms (min $($hello.min), max $($hello.max))"
"微软签名二进制  start→exit  中位 $($signed.median) ms (min $($signed.min), max $($signed.max))"
"veil.exe 墙钟   start→句柄  中位 $($veil.median) ms (min $($veil.min), max $($veil.max))"
