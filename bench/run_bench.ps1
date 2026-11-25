# Veil 固定基准：一条命令输出稳定可比的性能数字。
#
# 用法：
#   pwsh -NoProfile -File bench/run_bench.ps1                # 既有基准 + 击键 p99 补充
#   pwsh -NoProfile -File bench/run_bench.ps1 -WithProbe     # 额外跑全路径击键探针（release 测试，较慢）
#   pwsh -NoProfile -File bench/run_bench.ps1 -SkipBuild     # 复用已有构建产物
#
# 组成（不新增依赖）：
#   1. 既有例程 crates/veil/examples/bench_typing（唯一来源，本脚本只调用不复制）：
#      5005 块全量排版（冷启动口径）、核心击键 p50/p95、脏块重排、帧合成/复用、存活堆峰值。
#   2. 补充基准 bench/keystroke_p99.rs：与 bench_typing::measure_keystrokes 同口径，
#      但输出 p99/max（原例程只有 p50/p95，门禁需要 p99）。用 rustc 按 release profile
#      参数（opt-level=s / codegen-units=1 / panic=abort / lto=fat）直接编译，不新增 crate。
#   3. -WithProbe：既有 ignored 探针 perf_keystroke_path（真实路径 apply+after_edit+paint，
#      300 样本，输出 p50/p95/max；该探针不输出 p99，脚本不会用别的东西冒充）。
#
# 输出：
#   - 控制台打印 `bench.*` 归一化 key=value（供 tools/verify/perf_gate.ps1 解析）
#   - 原始日志归档到 bench/results/<UTC 时间戳>/（已被 .gitignore 忽略）
#
# 失败即非 0 退出（构建失败 / 产物缺失 / 数字缺失），不打印猜测值。

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$WithProbe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$resultsRoot = Join-Path $repoRoot 'bench\results'
$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
$outDir = Join-Path $resultsRoot $stamp
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

function Invoke-Step([string]$Label, [string]$Exe, [string[]]$Arguments) {
    Write-Host "==> $Label"
    $output = & $Exe @Arguments 2>&1
    $code = $LASTEXITCODE
    $output | ForEach-Object { Write-Host $_ }
    if ($code -ne 0) { throw "$Label 失败（exit $code）" }
    return @($output | ForEach-Object { [string]$_ })
}

# ---------- 1. 构建既有基准例程与 release 库 ----------
$exampleExe = Join-Path $repoRoot 'target\release\examples\bench_typing.exe'
$depsDir = Join-Path $repoRoot 'target\release\deps'
if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        $out = & cargo build --release -p veil --example bench_typing 2>&1
        $code = $LASTEXITCODE
        $out | ForEach-Object { Write-Host $_ }
        if ($code -ne 0) { throw "cargo build --example bench_typing 失败（exit $code）" }
    } finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $exampleExe)) { throw "缺少 $exampleExe，请先不跳过构建" }

# ---------- 2. 既有 bench_typing ----------
$lines = Invoke-Step 'bench_typing（既有基准）' $exampleExe @()
$lines | Set-Content -LiteralPath (Join-Path $outDir 'bench_typing.txt') -Encoding utf8

$typing = @{}
foreach ($line in $lines) {
    if ($line -match '^([A-Za-z0-9_]+)=(.+)$') { $typing[$Matches[1]] = $Matches[2] }
}
function Split-Pair([string]$Name) {
    if (-not $typing.ContainsKey($Name)) { throw "bench_typing 输出缺少 $Name" }
    $p = $typing[$Name] -split '/'
    if ($p.Count -ne 2) { throw "bench_typing 输出 $Name 格式异常：$($typing[$Name])" }
    return @([double]$p[0], [double]$p[1])
}
$cold = Split-Pair 'cold_start_ms'
$key = Split-Pair 'keystroke_core_us'
$dirty = Split-Pair 'layout_dirty_us'
$fa = Split-Pair 'frame_alloc_ms'
$fr = Split-Pair 'frame_reuse_ms'

$normalized = [ordered]@{
    'bench.block_count'              = $typing['block_count']
    'bench.cold_start_p50_ms'        = $cold[0]
    'bench.cold_start_max_ms'        = $cold[1]
    'bench.keystroke_core_p50_us'    = $key[0]
    'bench.keystroke_core_p95_us'    = $key[1]
    'bench.layout_dirty_p50_us'      = $dirty[0]
    'bench.layout_dirty_p95_us'      = $dirty[1]
    'bench.frame_alloc_p50_ms'       = $fa[0]
    'bench.frame_alloc_p95_ms'       = $fa[1]
    'bench.frame_reuse_p50_ms'       = $fr[0]
    'bench.frame_reuse_p95_ms'       = $fr[1]
    'bench.peak_heap_mb'             = $typing['peak_heap_mb']
}

# ---------- 3. 击键 p99 补充基准 ----------
$rlib = Get-ChildItem -LiteralPath $depsDir -Filter 'libveil_core-*.rlib' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($null -eq $rlib) { throw "缺少 libveil_core rlib（$depsDir），请先构建 release" }

$benchOutDir = Join-Path $repoRoot 'target\bench'
New-Item -ItemType Directory -Path $benchOutDir -Force | Out-Null
$p99Exe = Join-Path $benchOutDir 'keystroke_p99.exe'
$src = Join-Path $PSScriptRoot 'keystroke_p99.rs'
Write-Host "==> 编译 keystroke_p99（rustc, release profile 参数）"
$rustcOut = & rustc --edition 2021 -C opt-level=s -C codegen-units=1 -C panic=abort -C lto=fat `
    -L $depsDir --extern "veil_core=$($rlib.FullName)" $src -o $p99Exe 2>&1
$rustcCode = $LASTEXITCODE
if ($rustcOut) { $rustcOut | ForEach-Object { Write-Host $_ } }
if ($rustcCode -ne 0) { throw "rustc 编译 keystroke_p99 失败（exit $rustcCode）" }

$p99Lines = Invoke-Step 'keystroke_p99（补充基准）' $p99Exe @()
$p99Lines | Set-Content -LiteralPath (Join-Path $outDir 'keystroke_p99.txt') -Encoding utf8
$p99 = @{}
foreach ($line in $p99Lines) {
    if ($line -match '^([A-Za-z0-9_]+)=(.+)$') { $p99[$Matches[1]] = $Matches[2] }
}
foreach ($need in @('keystroke_core_p99_us', 'keystroke_core_max_us')) {
    if (-not $p99.ContainsKey($need)) { throw "keystroke_p99 输出缺少 $need" }
}
$normalized['bench.keystroke_core_p99_us'] = [double]$p99['keystroke_core_p99_us']
$normalized['bench.keystroke_core_max_us'] = [double]$p99['keystroke_core_max_us']
if ($p99.ContainsKey('keystroke_core_doc_chars')) {
    $normalized['bench.keystroke_doc_chars'] = [int]$p99['keystroke_core_doc_chars']
}

# ---------- 4. 可选：既有全路径探针（输出 p50/p95/max，不含 p99）----------
if ($WithProbe) {
    Push-Location $repoRoot
    try {
        $probeLines = Invoke-Step 'perf_keystroke_path（既有 ignored 探针，真实路径）' `
            'cargo' @('test', '-p', 'veil-ui', '--release', '--lib', '--', '--ignored',
                      '--nocapture', '--test-threads=1', 'perf_keystroke_path')
    } finally { Pop-Location }
    $probeLines | Set-Content -LiteralPath (Join-Path $outDir 'perf_keystroke_path.txt') -Encoding utf8
    $probe = @{}
    foreach ($line in $probeLines) {
        foreach ($m in [regex]::Matches($line, '(perf_[a-z0-9_]+)=([^\s]+)')) {
            $probe[$m.Groups[1].Value] = $m.Groups[2].Value
        }
    }
    if ($probe.ContainsKey('perf_keystroke_ms_p95')) {
        $normalized['bench.probe_keystroke_p50_ms'] = [double]$probe['perf_keystroke_ms_p50']
        $normalized['bench.probe_keystroke_p95_ms'] = [double]$probe['perf_keystroke_ms_p95']
        $normalized['bench.probe_keystroke_max_ms'] = [double]$probe['perf_keystroke_ms_max']
        $normalized['bench.probe_blocks'] = $probe['perf_blocks']
    } else {
        Write-Warning '探针输出未解析到 perf_keystroke_ms_p95，跳过该项'
    }
}

[ordered]@{
    time_utc  = $stamp
    raw_dir   = $outDir
    normalized = $normalized
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $outDir 'bench.json') -Encoding utf8

Write-Host ''
Write-Host "--- 归一化结果（原始日志：$outDir）---"
foreach ($k in $normalized.Keys) { Write-Host ("{0}={1}" -f $k, $normalized[$k]) }
