# Veil 性能门禁：逐条核对四项指标并写证据表。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/perf_gate.ps1                 # 完整门禁（含真实进程内存采样）
#   pwsh -NoProfile -File tools/verify/perf_gate.ps1 -WithProbe      # 额外跑既有全路径击键探针
#   pwsh -NoProfile -File tools/verify/perf_gate.ps1 -SkipGui        # 只跑进程内基准与体积
#
# 门禁（阈值写死，不允许为通过而放宽）：
#   1. 10 万字文档（5005 块）全量排版 < 100 ms
#      口径 = Session::open + 每块 layout_block 一次（bench_typing.cold_start_ms 中位，5 次），
#      **不是端到端首帧**。端到端首帧（进程创建 → 像素上屏）本脚本无采样手段，不冒充。
#   2. 击键 p99 < 16 ms
#      口径 = bench/keystroke_p99.rs（与 bench_typing::measure_keystrokes 同口径：5005 块、
#      1000 次单字符 Session::apply，最近秩 p99）。这是内核 apply 段；真实视图全路径的
#      p50/p95/max 由既有 ignored 探针 perf_keystroke_path 提供（该探针不输出 p99）。
#   3. 常驻内存 < 300 MB
#      口径 = 真实 veil.exe 打开约 10 万字文档后，每 100 ms 采样 WorkingSet64（RSS）取峰值；
#      同时记录 PrivateMemorySize64 峰值作为参考。
#   4. 二进制体积 ≤ 14,680,064 B
#      口径 = target\release\veil.exe 实际字节数；基线 14,135,296 B。
#
# 任一未达标或不可测 → exit 1，并打印是哪一项、差多少。
# 结果写入 evidence/verify/performance.md 的自动区块（marker 之间整体替换）。

[CmdletBinding()]
param(
    [switch]$SkipGui,
    [switch]$WithProbe,
    [switch]$SkipBuild,
    [string]$OutFile = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$exePath = Join-Path $repoRoot 'target\release\veil.exe'
if (-not $OutFile) { $OutFile = Join-Path $repoRoot 'evidence\verify\performance.md' }

$sizeLimit = 14680064L
$sizeBaseline = 14135296L
$fullLayoutLimitMs = 100.0
$keystrokeP99LimitMs = 16.0
$rssLimitMb = 300.0

$checks = [System.Collections.Generic.List[object]]::new()
$extras = [System.Collections.Generic.List[string]]::new()
$script:unmeasurable = [System.Collections.Generic.List[string]]::new()

function Add-Check {
    param(
        [string]$Item, [string]$Measured, [string]$Threshold, [string]$Scope,
        [bool]$Pass, [string]$Delta, [string]$Status = ''
    )
    $statusText = if ($Status) { $Status } elseif ($Pass) { 'PASS' } else { 'FAIL' }
    $script:checks.Add([pscustomobject]@{
            Item = $Item; Measured = $Measured; Threshold = $Threshold
            Scope = $Scope; Pass = $Pass; Delta = $Delta; Status = $statusText
        })
}

function Add-Unmeasurable {
    param([string]$Item, [string]$Threshold, [string]$Scope, [string]$Reason)
    Add-Check -Item $Item -Measured '不可测' -Threshold $Threshold -Scope $Scope -Pass $false `
        -Delta "不可测：$Reason" -Status 'UNMEASURED'
    $script:unmeasurable.Add("$Item：$Reason")
}

Write-Host '=== Veil 性能门禁 ==='
if (-not (Test-Path -LiteralPath $exePath)) {
    throw "找不到 $exePath，请先运行 cargo build --release --bin veil"
}

# ---------- 1/2. 进程内基准 ----------
Write-Host '--- 运行 bench/run_bench.ps1（既有基准 + 击键 p99）---'
$benchArgs = @('-NoProfile', '-File', (Join-Path $repoRoot 'bench\run_bench.ps1'), '-SkipBuild')
if ($WithProbe) { $benchArgs += '-WithProbe' }
if (-not $SkipBuild) { $benchArgs = $benchArgs | Where-Object { $_ -ne '-SkipBuild' } }
$benchOut = & pwsh @benchArgs 2>&1
$benchCode = $LASTEXITCODE
$benchOut | ForEach-Object { Write-Host $_ }
if ($benchCode -ne 0) { throw "bench/run_bench.ps1 失败（exit $benchCode），无法取数" }

$bench = @{}
foreach ($line in $benchOut) {
    $s = [string]$line
    if ($s -match '^bench\.([A-Za-z0-9_]+)=(.+)$') { $bench[$Matches[1]] = $Matches[2] }
}

# 1. 5005 块全量排版
if ($bench.ContainsKey('cold_start_p50_ms')) {
    $layoutMs = [double]$bench['cold_start_p50_ms']
    $pass = $layoutMs -lt $fullLayoutLimitMs
    $delta = if ($pass) { "余量 $([math]::Round($fullLayoutLimitMs - $layoutMs, 2)) ms" }
    else { "超出 $([math]::Round($layoutMs - $fullLayoutLimitMs, 2)) ms" }
    $layoutScope = 'Session::open + 每块 layout_block 一次，5 次中位（bench_typing.cold_start_ms）；非端到端首帧'
    if ($bench.ContainsKey('keystroke_doc_chars')) {
        $layoutScope += "；文档 5005 块 / $($bench['keystroke_doc_chars']) 字符"
    }
    Add-Check -Item '10 万字文档（5005 块）全量排版' -Measured "$([math]::Round($layoutMs, 2)) ms" `
        -Threshold '< 100 ms' -Scope $layoutScope -Pass $pass -Delta $delta
    Add-Check -Item '(参考) 同一轮 5005 块全量排版最大值' -Measured "$([math]::Round([double]$bench['cold_start_max_ms'], 2)) ms" `
        -Threshold '—' -Scope '同一基准 5 次的最大值' -Pass $true -Delta '参考值，不参与判定' -Status 'INFO'
} else {
    Add-Unmeasurable -Item '10 万字文档（5005 块）全量排版' -Threshold '< 100 ms' `
        -Scope 'bench_typing.cold_start_ms' -Reason '基准输出缺失 cold_start_ms'
}

# 2. 击键 p99
if ($bench.ContainsKey('keystroke_core_p99_us')) {
    $p99Ms = [double]$bench['keystroke_core_p99_us'] / 1000.0
    $pass = $p99Ms -lt $keystrokeP99LimitMs
    $delta = if ($pass) { "余量 $([math]::Round($keystrokeP99LimitMs - $p99Ms, 3)) ms" }
    else { "超出 $([math]::Round($p99Ms - $keystrokeP99LimitMs, 3)) ms" }
    Add-Check -Item '击键 p99' -Measured "$([math]::Round($p99Ms, 3)) ms" -Threshold '< 16 ms' `
        -Scope '5005 块 / 1000 次单字符 Session::apply / 最近秩 p99（bench/keystroke_p99.rs，同 bench_typing 口径）' `
        -Pass $pass -Delta $delta
    Add-Check -Item '(参考) 击键 p50 / p95' `
        -Measured "$([math]::Round([double]$bench['keystroke_core_p50_us'] / 1000.0, 3)) / $([math]::Round([double]$bench['keystroke_core_p95_us'] / 1000.0, 3)) ms" `
        -Threshold '—' -Scope '同基准核内 apply 段' -Pass $true -Delta '参考值，不参与判定' -Status 'INFO'
    Add-Check -Item '(参考) 击键峰值 max' -Measured "$([math]::Round([double]$bench['keystroke_core_max_us'] / 1000.0, 3)) ms" `
        -Threshold '—' -Scope '同基准 1000 样本最大' -Pass $true -Delta '参考值，不参与判定' -Status 'INFO'
} else {
    Add-Unmeasurable -Item '击键 p99' -Threshold '< 16 ms' `
        -Scope 'bench/keystroke_p99.rs' -Reason '基准输出缺失 keystroke_core_p99_us'
}

if ($bench.ContainsKey('probe_keystroke_p95_ms')) {
    $extras.Add("- 既有全路径探针 `perf_keystroke_path`（apply + after_edit + paint，300 样本）：" +
        "p50 $([math]::Round([double]$bench['probe_keystroke_p50_ms'], 3)) ms / " +
        "p95 $([math]::Round([double]$bench['probe_keystroke_p95_ms'], 3)) ms / " +
        "max $([math]::Round([double]$bench['probe_keystroke_max_ms'], 3)) ms，" +
        "块数 $($bench['probe_blocks'])；该探针只输出 p50/p95/max，不输出 p99。")
} else {
    $extras.Add('- 既有全路径探针 `perf_keystroke_path` 本轮未运行（加 `-WithProbe` 可运行；它只输出 p50/p95/max，无 p99）。')
}
$extras.Add("- 同轮的脏块重排 p50/p95：$([math]::Round([double]$bench['layout_dirty_p50_us'], 2)) / $([math]::Round([double]$bench['layout_dirty_p95_us'], 2)) µs；" +
    "帧分配 p50 $([math]::Round([double]$bench['frame_alloc_p50_ms'], 2)) ms vs 复用 p50 $([math]::Round([double]$bench['frame_reuse_p50_ms'], 2)) ms；" +
    "存活堆峰值 $($bench['peak_heap_mb']) MB。")
if ($bench.ContainsKey('keystroke_doc_chars')) {
    $extras.Add("- 5005 块基准文档体量（同一构造，实测）：$($bench['keystroke_doc_chars']) 字符。")
}

# ---------- 3. 常驻内存（真实进程，峰值 RSS）----------
$startupMs = $null
$rssPeakMb = $null
$privatePeakMb = $null
$docChars = 0
$docBytes = 0
if ($SkipGui) {
    Add-Unmeasurable -Item '常驻内存（峰值 RSS）' -Threshold '< 300 MB' `
        -Scope '真实 veil.exe + 约 10 万字文档，WorkingSet64 峰值' -Reason '调用方指定 -SkipGui'
} else {
    $tmpRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-perf-gate-{0}-{1}" -f (Get-Date -Format 'yyyyMMdd-HHmmssfff'), $PID)
    $proc = $null
    try {
        $homeDir = Join-Path $tmpRoot 'home'
        $appData = Join-Path $tmpRoot 'appdata'
        $localAppData = Join-Path $tmpRoot 'localappdata'
        $tempDir = Join-Path $tmpRoot 'temp'
        New-Item -ItemType Directory -Path $homeDir, $appData, $localAppData, $tempDir -Force | Out-Null

        # 约 10 万字 / 约 4800 块的文档（结构同 tools/bench/gen_corpus.py 的 large 档）
        $sb = [System.Text.StringBuilder]::new(300000)
        [void]$sb.Append("# Veil 性能门禁语料`n`n")
        $fence = '```'
        $sections = 520
        for ($i = 1; $i -le $sections; $i++) {
            [void]$sb.Append("## 第 $i 节 标题`n`n")
            [void]$sb.Append("写作时最重要的是不被工具打断思路。渲染可以失之毫厘，但字节流必须分毫不差。`n`n")
            [void]$sb.Append("- 列表项 $i-1：本地优先意味着数据始终在自己手中。`n- 列表项 $i-2：好的工具是透明的。`n`n")
            [void]$sb.Append("> 引用段落 $i：当文档增长到十万字量级，编辑器仍然保持跟手才算合格。`n`n")
            [void]$sb.Append($fence + 'rust' + "`n" + "fn section_$i() -> u32 { $i }`n" + $fence + "`n`n")
            [void]$sb.Append("---`n`n")
        }
        $docText = $sb.ToString()
        $docChars = $docText.Length
        $docPath = Join-Path $tmpRoot 'perf-gate-100k.md'
        [IO.File]::WriteAllText($docPath, $docText, [Text.UTF8Encoding]::new($false))
        $docBytes = (Get-Item -LiteralPath $docPath).Length
        Write-Host "--- 内存采样：文档 $docChars 字符 / $docBytes 字节 ---"

        $sw = [Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process -FilePath $exePath -ArgumentList @($docPath) -PassThru -Environment @{
            VEIL_HOME = $homeDir; APPDATA = $appData; LOCALAPPDATA = $localAppData
            TEMP = $tempDir; TMP = $tempDir
        }
        while (-not $proc.HasExited -and $proc.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 20000) {
            Start-Sleep -Milliseconds 10
            $proc.Refresh()
        }
        if ($proc.HasExited) { throw "进程在窗口出现前退出（ExitCode=$($proc.ExitCode)）" }
        if ($proc.MainWindowHandle -eq 0) { throw '等待主窗口超时（20s）' }
        $startupMs = $sw.ElapsedMilliseconds

        $rssSamples = [System.Collections.Generic.List[double]]::new()
        $privSamples = [System.Collections.Generic.List[double]]::new()
        for ($n = 0; $n -lt 100; $n++) {
            Start-Sleep -Milliseconds 100
            if ($proc.HasExited) { throw "采样期间进程退出（第 $n 次，ExitCode=$($proc.ExitCode)）" }
            $proc.Refresh()
            $rssSamples.Add($proc.WorkingSet64 / 1MB)
            $privSamples.Add($proc.PrivateMemorySize64 / 1MB)
        }
        $respond = $proc.Responding
        $rssPeakMb = ($rssSamples | Measure-Object -Maximum).Maximum
        $privatePeakMb = ($privSamples | Measure-Object -Maximum).Maximum
        $extras.Add("- 内存采样：10 s / 100 次（每 100 ms），最终 Responding=$respond；" +
            "Private 峰值 $([math]::Round($privatePeakMb, 2)) MB（参考，非判定口径）。")

        if ($rssPeakMb -lt $rssLimitMb) {
            Add-Check -Item '常驻内存（峰值 RSS）' -Measured "$([math]::Round($rssPeakMb, 2)) MB" -Threshold '< 300 MB' `
                -Scope "真实 veil.exe 打开 $docChars 字符文档，WorkingSet64 10 s 峰值" `
                -Pass $true -Delta "余量 $([math]::Round($rssLimitMb - $rssPeakMb, 2)) MB"
        } else {
            Add-Check -Item '常驻内存（峰值 RSS）' -Measured "$([math]::Round($rssPeakMb, 2)) MB" -Threshold '< 300 MB' `
                -Scope "真实 veil.exe 打开 $docChars 字符文档，WorkingSet64 10 s 峰值" `
                -Pass $false -Delta "超出 $([math]::Round($rssPeakMb - $rssLimitMb, 2)) MB"
        }
        Add-Check -Item '(参考) 冷启动到窗口句柄' -Measured "$startupMs ms" -Threshold '—' `
            -Scope '进程创建 → MainWindowHandle 出现；单次测量、波动大；**不是端到端首帧**' -Pass $true `
            -Delta '参考值，不参与判定' -Status 'INFO'
    } catch {
        Add-Unmeasurable -Item '常驻内存（峰值 RSS）' -Threshold '< 300 MB' `
            -Scope '真实 veil.exe + 约 10 万字文档，WorkingSet64 峰值' -Reason $_.Exception.Message
    } finally {
        if ($null -ne $proc) {
            try { if (-not $proc.HasExited) { $proc.Kill(); $proc.WaitForExit(5000) | Out-Null } } catch { }
        }
        try { if (Test-Path -LiteralPath $tmpRoot) { Remove-Item -LiteralPath $tmpRoot -Recurse -Force } } catch { }
    }
}

# ---------- 4. 二进制体积 ----------
$sizeBytes = (Get-Item -LiteralPath $exePath).Length
$sizePass = $sizeBytes -le $sizeLimit
$sizeDelta = if ($sizePass) {
    "余量 $($sizeLimit - $sizeBytes) B（相对基线 $($sizeBytes - $sizeBaseline) B）"
} else {
    "超出 $($sizeBytes - $sizeLimit) B"
}
Add-Check -Item '二进制体积' -Measured "$sizeBytes B" -Threshold '≤ 14,680,064 B' `
    -Scope 'target\release\veil.exe 实际字节；基线 14,135,296 B' -Pass $sizePass -Delta $sizeDelta

# ---------- 汇总 ----------
Write-Host ''
$allPass = $true
foreach ($c in $checks) {
    if (-not $c.Pass -and $c.Status -ne 'INFO') { $allPass = $false }
    Write-Host ("[{0}] {1}：实测 {2} / 阈值 {3} / {4}" -f $c.Status, $c.Item, $c.Measured, $c.Threshold, $c.Delta)
}
Write-Host ''
Write-Host $(if ($allPass) { '门禁结果：PASS' } else { '门禁结果：FAIL' })

# ---------- 写证据 ----------
$machine = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$ramGb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
$os = (Get-CimInstance Win32_OperatingSystem)
$osText = "$($os.Caption) $($os.Version)"
$rustcVersion = (& rustc -V) -join ''
$cargoToml = Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Raw
$profileLines = ($cargoToml -split '\[profile\.release\]')[1]
$profileText = (($profileLines -split "`n") | Where-Object { $_ -match '^\s*(lto|codegen-units|panic|strip|opt-level)\s*=' } |
    ForEach-Object { $_.Trim() }) -join ', '

$stamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$rows = foreach ($c in $checks) {
    $verdict = switch ($c.Status) { 'INFO' { '参考' } 'UNMEASURED' { '**不可测**' } 'FAIL' { '**FAIL**' } default { 'PASS' } }
    "| $($c.Item) | $($c.Measured) | $($c.Threshold) | $($c.Scope) | $verdict | $($c.Delta) |"
}
$extrasText = ($extras | ForEach-Object { $_ }) -join "`n"
$unmeasuredText = if ($script:unmeasurable.Count -gt 0) {
    ($script:unmeasurable | ForEach-Object { "- $_" }) -join "`n"
} else { '- 无' }

$block = @"
<!-- perf-gate:begin（本区块由 tools/verify/perf_gate.ps1 生成，重复运行整体替换） -->
## 自动门禁结果

- 时间：$stamp
- 机器：$machine / $ramGb GB RAM / $osText
- 工具链：$rustcVersion
- 构建类型：release（profile：$profileText）
- 二进制：``target\release\veil.exe`` $sizeBytes B（基线 $sizeBaseline B）
- 运行命令：``pwsh -NoProfile -File tools/verify/perf_gate.ps1``

### 四项门禁

| 项 | 实测 | 阈值 | 口径 | 判定 | 差额 |
|---|---|---|---|---|---|
$($rows -join "`n")

### 参考数字（不参与判定）

$extrasText

### 不可测项

$unmeasuredText

### 口径辨析（必读）

- **「5005 块全量排版」不是「端到端首帧」**：前者是进程内 Session::open 后把全部 5005 个块各排版一次的中位耗时，
  是 bench_typing 既有基准的冷启动口径；后者是进程创建到像素真正上屏，本脚本没有屏幕像素时间戳采样，
  **不测、也不拿别的东西冒充**。上表「冷启动到窗口句柄」这一项同样不是端到端首帧。
- **击键 p99 的口径是内核 apply 段**（5005 块、1000 次单字符插入、最近秩 p99），与 bench_typing 同源；
  真实视图全路径（apply + refresh_index + 脏块排版 + 光栅 + 帧合成）的既有探针只输出 p50/p95/max，
  未加 ``-WithProbe`` 时则没有全路径数字，且该探针**没有 p99**。
- **常驻内存用峰值 RSS（WorkingSet64）**：真实 veil.exe 打开约 10 万字文档后每 100 ms 采样 10 s 取峰值；
  Private（私有提交）仅作参考，不是判定口径。
- 体积以 ``target\release\veil.exe`` 实际字节为准，阈值 14,680,064 B 与基线 14,135,296 B 均为固定常量。

### 本次判定

- 结果：$(if ($allPass) { '**PASS**（四项全部达标或参考项已标注）' } else { '**FAIL**（见上表 FAIL/不可测项）' })
- 脚本退出码：$(if ($allPass) { 0 } else { 1 })
<!-- perf-gate:end -->
"@

$dir = Split-Path -Parent $OutFile
New-Item -ItemType Directory -Path $dir -Force | Out-Null
if (Test-Path -LiteralPath $OutFile) {
    $existing = Get-Content -LiteralPath $OutFile -Raw
    if ($existing -match '(?s)<!-- perf-gate:begin.*?<!-- perf-gate:end -->') {
        $existing = [regex]::Replace($existing, '(?s)<!-- perf-gate:begin.*?<!-- perf-gate:end -->', $block.Replace('$', '$$'))
        [IO.File]::WriteAllText($OutFile, $existing, [Text.UTF8Encoding]::new($false))
    } else {
        [IO.File]::WriteAllText($OutFile, ($existing.TrimEnd() + "`n`n" + $block + "`n"), [Text.UTF8Encoding]::new($false))
    }
} else {
    [IO.File]::WriteAllText($OutFile, ("# Veil 性能门禁（本轮实测）`n`n" + $block + "`n"), [Text.UTF8Encoding]::new($false))
}
Write-Host "证据已写入：$OutFile"

if ($allPass) { exit 0 } else { exit 1 }
