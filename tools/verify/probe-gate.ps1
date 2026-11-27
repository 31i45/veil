# Veil ignored 探针门禁：单命令在 release 构建下逐条运行 17 条 #[ignore] 探针
# （其中 pathological_eval_child 以 loop / microtask 两种模式各跑一次），
# 解析探针输出的机器可读 `PROBE name=... metric=... value=... unit=...` 行，
# 按归档基线与探针自身预算判定，并生成结果归档文档。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/probe-gate.ps1                 # 默认 1 次采样
#   pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 3       # 性能项 3 次采样，取 p95
#   pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat 9       # Task 5 的分布口径
#   pwsh -NoProfile -File tools/verify/probe-gate.ps1 -UpdateBaseline # 用本轮结果重写基线
#
# 判定：
#   - 性能项：本轮 p95 > 归档基线 p95 × 1.2 或 > 探针登记预算 → FAIL；
#     基线文件不存在（首次）或 -UpdateBaseline 时写入，不判 FAIL。
#   - 非性能项：退出码 + 关键不变量（PROBE 指标 / 证据文件存在且非空）；
#     缺少系统 CJK 字体等环境缺失记 SKIP 并写明原因，绝不记 PASS。
#   - 任一探针超时（默认 10 分钟）记 FAIL 并保留日志摘要。
#   - 任一 FAIL → 退出码 1。
#
# 结果写入：
#   docs/qa/probe-gate-<yyyy-mm-dd>.md   逐条 PASS/FAIL/SKIP + 原因 + 耗时 + 指标
#   docs/qa/perf-baseline.json           性能基线（首次或 -UpdateBaseline 写入）

[CmdletBinding()]
param(
    [switch]$UpdateBaseline,
    [int]$Repeat = 1
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Repeat -lt 1) { throw '-Repeat 必须 >= 1' }

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$dateStamp = Get-Date -Format 'yyyy-MM-dd'
$timestamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$archivePath = Join-Path $repoRoot "docs\qa\probe-gate-$dateStamp.md"
$baselinePath = Join-Path $repoRoot 'docs\qa\perf-baseline.json'
$cargo = (Get-Command cargo -ErrorAction Stop).Source
$regressionFactor = 1.2
$defaultTimeoutSec = 600

# 探针清单。Test 为 cargo test 报告的全限定名（--exact 必须完全一致）。
$probes = @(
    # ---------- 病态 / 诊断 / 证据 ----------
    @{
        Id = 'pathological_eval_child/loop'; Kind = '病态'
        Target = @('-p', 'veil-mermaid-offline', '--lib'); Test = 'tests::pathological_eval_child'
        Ignored = $true; TimeoutSec = 300; Env = @{ VEIL_PATHOLOGICAL_EVAL = 'loop' }
        ExactMetrics = @{ answer = 42 }; MinMetrics = @{}; GateMetrics = @(); Budgets = @{}
    },
    @{
        Id = 'pathological_eval_child/microtask'; Kind = '病态'
        Target = @('-p', 'veil-mermaid-offline', '--lib'); Test = 'tests::pathological_eval_child'
        Ignored = $true; TimeoutSec = 300; Env = @{ VEIL_PATHOLOGICAL_EVAL = 'microtask' }
        ExactMetrics = @{ answer = 42 }; MinMetrics = @{}; GateMetrics = @(); Budgets = @{}
    },
    @{
        Id = 'pathological_inputs_matrix'; Kind = '病态'
        Target = @('-p', 'veil-core', '--test', 'pathological'); Test = 'pathological_inputs_matrix'
        Ignored = $true; TimeoutSec = 900; Env = @{}
        ExactMetrics = @{ cases_passed = 10 }; MinMetrics = @{}; GateMetrics = @(); Budgets = @{}
    },
    @{
        Id = 'mindmap_viewbox_debug'; Kind = '诊断'
        Target = @('-p', 'veil-mermaid-offline', '--lib'); Test = 'tests::mindmap_viewbox_debug'
        Ignored = $false; TimeoutSec = 300; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{ viewbox_width = 1; viewbox_height = 1 }
        GateMetrics = @(); Budgets = @{}
    },
    @{
        Id = 'generate_wave4_sample_pdf'; Kind = '证据'
        Target = @('-p', 'veil', '--lib'); Test = 'platform::pdf_layout::tests::generate_wave4_sample_pdf'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{ pages = 3; bytes = 1 }; GateMetrics = @(); Budgets = @{}
        EvidenceFile = 'target/wave4-sample.pdf'; SkipPattern = '系统 CJK 字体|find_cjk_font'
    },
    @{
        Id = 'perf_huge_scroll'; Kind = '病态'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_huge_scroll'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{ nonbg = 1001 }; GateMetrics = @(); Budgets = @{}
    },
    @{
        Id = 'perf_huge_block'; Kind = '病态'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_huge_block'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @(); Budgets = @{}
    },
    # ---------- 性能 ----------
    @{
        Id = 'bench_edit_reparse_history_100k'; Kind = '性能'
        Target = @('-p', 'veil-core', '--lib'); Test = 'session::tests::bench_edit_reparse_history_100k'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('reparse_us', 'edit_us'); Budgets = @{}
    },
    @{
        Id = 'perf_50mb_long_doc'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::persist::perf_50mb_long_doc'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}
        GateMetrics = @('keystroke_p95', 'open_ms'); Budgets = @{ keystroke_p95 = 16.0; open_ms = 5000.0 }
    },
    @{
        Id = 'perf_20k_blocks'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::persist::perf_20k_blocks'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('keystroke_p95'); Budgets = @{}
    },
    @{
        Id = 'perf_keystroke_path'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_keystroke_path'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('keystroke_p95'); Budgets = @{}
    },
    @{
        Id = 'perf_keystroke_attribution'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_keystroke_attribution'
        Ignored = $true; TimeoutSec = 900; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('total_p95'); Budgets = @{}
    },
    @{
        Id = 'perf_index_subsegments'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_index_subsegments'
        Ignored = $true; TimeoutSec = 900; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('index_sum_p95'); Budgets = @{}
    },
    @{
        Id = 'perf_cold_first_edit'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_cold_first_edit'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('cold_first_edit_ms'); Budgets = @{}
    },
    @{
        Id = 'perf_cold_first_edit_prewarmed'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_cold_first_edit_prewarmed'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('edit_ms'); Budgets = @{}
    },
    @{
        Id = 'perf_first_edit_after_first_frame'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::render::perf_first_edit_after_first_frame'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('first_edit_ms'); Budgets = @{}
    },
    @{
        Id = 'perf_peak_breakdown'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::view::perf_peak_breakdown'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}; GateMetrics = @('peak_total_ms'); Budgets = @{}
    },
    @{
        Id = 'perf_after_edit_breakdown'; Kind = '性能'
        Target = @('-p', 'veil-ui', '--lib'); Test = 'app_tests::view::perf_after_edit_breakdown'
        Ignored = $true; TimeoutSec = 600; Env = @{}
        ExactMetrics = @{}; MinMetrics = @{}
        GateMetrics = @('peak_total_ms', 'p95_refresh_index_ms'); Budgets = @{}
    }
)

function Get-Percentile {
    param([double[]]$Values, [double]$Q)
    if ($null -eq $Values -or $Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $idx = [int][math]::Ceiling($sorted.Count * $Q) - 1
    if ($idx -lt 0) { $idx = 0 }
    if ($idx -ge $sorted.Count) { $idx = $sorted.Count - 1 }
    return [double]$sorted[$idx]
}

function Format-MetricValue {
    param([double]$Value)
    if ([math]::Abs($Value) -ge 1000) { return [string][math]::Round($Value, 1) }
    return [string][math]::Round($Value, 3)
}

# 解析 `PROBE name=<n> metric=<m> value=<v> unit=<u> [scope=<s>]` 行。
function Read-ProbeLines {
    param([string]$Text)
    $result = [System.Collections.Generic.List[object]]::new()
    $pattern = 'PROBE\s+name=(\S+)\s+metric=(\S+)\s+value=(-?[0-9]+(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?)\s+unit=(\S+)(?:\s+scope=(\S+))?'
    foreach ($m in [regex]::Matches($Text, $pattern)) {
        $scope = if ($m.Groups[5].Success) { $m.Groups[5].Value } else { '' }
        $result.Add([pscustomobject]@{
                Name  = $m.Groups[1].Value
                Metric = $m.Groups[2].Value
                Value = [double]$m.Groups[3].Value
                Unit  = $m.Groups[4].Value
                Scope = $scope
            })
    }
    return $result
}

function New-BaselineKey {
    param([string]$Name, [string]$Metric, [string]$Scope)
    if ($Scope) { return "$Name/$Metric@$Scope" }
    return "$Name/$Metric"
}

# 单次 cargo test 运行（带超时）。返回 stdout / stderr / 退出码 / 是否超时。
function Invoke-ProbeRun {
    param($Probe, [int]$TimeoutSec)
    $argList = @('test') + $Probe.Target + @('--release', '--')
    if ($Probe.Ignored) { $argList += '--ignored' }
    $argList += @('--exact', $Probe.Test, '--nocapture')

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $cargo
    $psi.Arguments = ($argList -join ' ')
    $psi.WorkingDirectory = $repoRoot
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    if ($Probe.ContainsKey('Env') -and $Probe.Env.Count -gt 0) {
        foreach ($k in $Probe.Env.Keys) { $psi.EnvironmentVariables[$k] = [string]$Probe.Env[$k] }
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = [System.Diagnostics.Process]::new()
    $proc.StartInfo = $psi
    [void]$proc.Start()
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()
    $timedOut = $false
    if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
        $timedOut = $true
        try { $proc.Kill($true) } catch { }
        try { [void]$proc.WaitForExit(10000) } catch { }
    }
    $stdout = ''
    $stderr = ''
    try { $stdout = $outTask.GetAwaiter().GetResult() } catch { }
    try { $stderr = $errTask.GetAwaiter().GetResult() } catch { }
    $code = if ($timedOut) { -1 } else { $proc.ExitCode }
    $sw.Stop()
    $proc.Dispose()
    return [pscustomobject]@{
        Stdout = $stdout; Stderr = $stderr; ExitCode = $code
        TimedOut = $timedOut; DurationSec = [math]::Round($sw.Elapsed.TotalSeconds, 2)
    }
}

function Get-OutputExcerpt {
    param([string]$Text, [int]$MaxLines = 12)
    if (-not $Text) { return '' }
    $lines = @($Text -split "`r?`n" | Where-Object { $_.Trim().Length -gt 0 })
    if ($lines.Count -le $MaxLines) { return ($lines -join "`n") }
    return (($lines | Select-Object -Last $MaxLines) -join "`n")
}

# ---------- 1/3 构建 ----------
Write-Host '=== 1/3 release 测试二进制构建（cargo test --workspace --release --no-run）==='
$buildSw = [System.Diagnostics.Stopwatch]::StartNew()
$buildOutput = & $cargo test --workspace --release --no-run 2>&1
$buildCode = $LASTEXITCODE
$buildSw.Stop()
$buildSec = [math]::Round($buildSw.Elapsed.TotalSeconds, 1)
if ($buildCode -ne 0) {
    Write-Host (($buildOutput | Select-Object -Last 40) -join "`n")
    Write-Error "release 测试构建失败（exit $buildCode）"
    exit 1
}
Write-Host "构建完成：${buildSec}s"

# ---------- 2/3 逐条运行 ----------
$baselineExistedAtStart = Test-Path -LiteralPath $baselinePath
$baseline = @{}
if ($baselineExistedAtStart) {
    $raw = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
    foreach ($p in $raw.entries.PSObject.Properties) { $baseline[$p.Name] = $p.Value }
}
$baselineKeysAtStart = @($baseline.Keys)

$results = [System.Collections.Generic.List[object]]::new()
$baselineUpdates = @{}
$index = 0

foreach ($probe in $probes) {
    $index++
    $runCount = if ($probe.Kind -eq '性能') { $Repeat } else { 1 }
    $timeout = if ($probe.ContainsKey('TimeoutSec')) { $probe.TimeoutSec } else { $defaultTimeoutSec }
    Write-Host ("--- [{0}/{1}] {2}（{3}，{4} 次，超时 {5}s）---" -f $index, $probes.Count, $probe.Id, $probe.Kind, $runCount, $timeout)

    $runs = [System.Collections.Generic.List[object]]::new()
    $totalSec = 0.0
    for ($r = 1; $r -le $runCount; $r++) {
        $run = Invoke-ProbeRun -Probe $probe -TimeoutSec $timeout
        $runs.Add($run)
        $totalSec += $run.DurationSec
        $runOut = $run.Stdout + "`n" + $run.Stderr
        $runProbes = @(Read-ProbeLines -Text $runOut)
        Write-Host ("    第 {0} 次：exit={1} 超时={2} 耗时={3}s PROBE 行={4}" -f $r, $run.ExitCode, $run.TimedOut, $run.DurationSec, $runProbes.Count)
    }

    $combined = (($runs | ForEach-Object { $_.Stdout + "`n" + $_.Stderr }) -join "`n")
    $allProbeLines = @(Read-ProbeLines -Text $combined)
    $status = 'PASS'
    $reasons = [System.Collections.Generic.List[string]]::new()

    $timedOutRun = $runs | Where-Object { $_.TimedOut } | Select-Object -First 1
    $failedRun = $runs | Where-Object { -not $_.TimedOut -and $_.ExitCode -ne 0 } | Select-Object -First 1

    if ($null -ne $timedOutRun) {
        $status = 'FAIL'
        $reasons.Add("超时超过 ${timeout}s 未结束，已终止进程")
    } elseif ($null -ne $failedRun) {
        $failText = $failedRun.Stdout + "`n" + $failedRun.Stderr
        if ($probe.ContainsKey('SkipPattern') -and $failText -match $probe.SkipPattern) {
            $status = 'SKIP'
            $reasons.Add('环境缺失：缺少系统 CJK 字体（测试自身报错信息匹配 SkipPattern）')
        } elseif ($probe.Kind -eq '性能' -and $failText -match 'memory allocation of .* failed|out of memory|Cannot allocate memory') {
            $status = 'SKIP'
            $reasons.Add('资源不足：内存分配失败（探针无法完成，未伪造 PASS）')
        } else {
            $status = 'FAIL'
            $reasons.Add("退出码 $($failedRun.ExitCode)")
        }
    } else {
        # 退出码 0：确认测试真的执行了（--exact 名称不匹配时 cargo 会跑 0 个测试）
        if ($combined -notmatch 'test result: ok\. 1 passed' -or $combined -match 'running 0 tests') {
            $status = 'FAIL'
            $reasons.Add('未按名称执行到测试（--exact 名称不匹配或 0 passed）')
        }
    }

    # 关键不变量（每次运行都必须满足）
    if ($status -eq 'PASS') {
        foreach ($metricName in @($probe.ExactMetrics.Keys)) {
            $expected = $probe.ExactMetrics[$metricName]
            $lines = @($allProbeLines | Where-Object { $_.Metric -eq $metricName })
            if ($lines.Count -eq 0) {
                $status = 'FAIL'; $reasons.Add("缺少 PROBE 指标 $metricName")
            } elseif (@($lines | Where-Object { [math]::Abs($_.Value - $expected) -gt 0.0001 }).Count -gt 0) {
                $status = 'FAIL'; $reasons.Add("$metricName 期望 $expected，实测 $(($lines | ForEach-Object { $_.Value }) -join '/')")
            }
        }
        foreach ($metricName in @($probe.MinMetrics.Keys)) {
            $minimum = $probe.MinMetrics[$metricName]
            $lines = @($allProbeLines | Where-Object { $_.Metric -eq $metricName })
            if ($lines.Count -eq 0) {
                $status = 'FAIL'; $reasons.Add("缺少 PROBE 指标 $metricName")
            } elseif (@($lines | Where-Object { $_.Value -lt $minimum }).Count -gt 0) {
                $status = 'FAIL'; $reasons.Add("$metricName 低于下限 $minimum")
            }
        }
        if ($probe.ContainsKey('EvidenceFile')) {
            $evidence = Join-Path $repoRoot $probe.EvidenceFile
            if (-not (Test-Path -LiteralPath $evidence)) {
                $status = 'FAIL'; $reasons.Add("证据文件不存在：$($probe.EvidenceFile)")
            } elseif ((Get-Item -LiteralPath $evidence).Length -le 0) {
                $status = 'FAIL'; $reasons.Add("证据文件为空：$($probe.EvidenceFile)")
            }
        }
    }

    # 记录本探针输出的全部 PROBE 指标（min/p50/p95/max），供归档文档与人工复核。
    $metricTexts = [System.Collections.Generic.List[string]]::new()
    if ($allProbeLines.Count -gt 0) {
        $allGroups = $allProbeLines | Group-Object -Property {
            if ($_.Scope) { "$($_.Metric)@$($_.Scope)" } else { $_.Metric }
        }
        foreach ($group in $allGroups) {
            $allValues = @($group.Group | ForEach-Object { $_.Value })
            $allUnit = $group.Group[0].Unit
            $p95All = Get-Percentile -Values $allValues -Q 0.95
            $p50All = Get-Percentile -Values $allValues -Q 0.50
            $minAll = ($allValues | Measure-Object -Minimum).Minimum
            $maxAll = ($allValues | Measure-Object -Maximum).Maximum
            $metricTexts.Add(("{0}: {1}/{2}/{3}/{4} {5}（min/p50/p95/max）" -f $group.Name,
                    (Format-MetricValue $minAll), (Format-MetricValue $p50All),
                    (Format-MetricValue $p95All), (Format-MetricValue $maxAll), $allUnit))
        }
    }

    # 性能项：与归档基线 / 探针预算比较。
    if ($status -eq 'PASS' -and $probe.GateMetrics.Count -gt 0) {
        foreach ($metricName in $probe.GateMetrics) {
            $keys = @($allProbeLines | Where-Object { $_.Metric -eq $metricName })
            if ($keys.Count -eq 0) {
                $status = 'FAIL'; $reasons.Add("未输出性能指标 $metricName 的 PROBE 行")
                continue
            }
            # 同一 metric 可能按 scope 分多组（如 5 档文档规模）；逐组判定。
            $groups = $keys | Group-Object -Property Scope
            foreach ($group in $groups) {
                $values = @($group.Group | ForEach-Object { $_.Value })
                $unit = $group.Group[0].Unit
                $scope = $group.Name
                $p95 = Get-Percentile -Values $values -Q 0.95
                $key = New-BaselineKey -Name $probe.Id -Metric $metricName -Scope $scope
                $scopeLabel = if ($scope) { "$metricName@$scope" } else { $metricName }

                $budget = if ($probe.Budgets.ContainsKey($metricName)) { [double]$probe.Budgets[$metricName] } else { $null }
                $hadBaseline = $baselineKeysAtStart -contains $key
                if ($null -ne $budget -and $p95 -gt $budget) {
                    $status = 'FAIL'
                    $reasons.Add("$scopeLabel p95=$(Format-MetricValue $p95) $unit 超过探针预算 $budget $unit")
                } elseif ($hadBaseline -and -not $UpdateBaseline) {
                    $baseValue = [double]$baseline[$key].value
                    $limit = $baseValue * $regressionFactor
                    if ($p95 -gt $limit) {
                        $status = 'FAIL'
                        $ratio = if ($baseValue -gt 0) { [math]::Round(($p95 / $baseValue - 1) * 100, 1) } else { 0 }
                        $reasons.Add("$scopeLabel p95=$(Format-MetricValue $p95) $unit 超基线 $(Format-MetricValue $baseValue) $unit 的 20%（+$ratio%）")
                    } else {
                        $reasons.Add("$scopeLabel p95 在基线 $(Format-MetricValue $baseValue) $unit 的 +20% 内")
                    }
                }

                # 只在成功的性能运行上写基线，避免把超时/失败值写进基线。
                if ($status -ne 'FAIL') {
                    if ($UpdateBaseline -or -not $hadBaseline) {
                        $baselineUpdates[$key] = [ordered]@{
                            value   = [math]::Round($p95, 4)
                            unit    = $unit
                            budget  = $budget
                            samples = @($values | ForEach-Object { [math]::Round($_, 4) })
                            repeat  = $values.Count
                            updated = $timestamp
                        }
                        $baseline[$key] = [pscustomobject]$baselineUpdates[$key]
                    }
                }
            }
        }
    }

    $results.Add([pscustomobject]@{
            Id = $probe.Id; Kind = $probe.Kind; Test = $probe.Test
            Status = $status; DurationSec = [math]::Round($totalSec, 2)
            Metrics = ($metricTexts -join '；')
            Reason = ($reasons -join '；')
            Excerpt = if ($status -eq 'FAIL') { Get-OutputExcerpt -Text $combined } else { '' }
        })
    Write-Host ("    => {0}（{1}s）{2}" -f $status, [math]::Round($totalSec, 2), ($reasons -join '；'))
}

# ---------- 基线落盘 ----------
if ($baselineUpdates.Count -gt 0) {
    $ordered = [ordered]@{}
    foreach ($key in ($baseline.Keys | Sort-Object)) {
        $entry = $baseline[$key]
        $ordered[$key] = [ordered]@{
            value   = $entry.value
            unit    = $entry.unit
            budget  = $entry.budget
            samples = $entry.samples
            repeat  = $entry.repeat
            updated = $entry.updated
        }
    }
    $machine = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
    $payload = [ordered]@{
        schema    = 1
        note      = '由 tools/verify/probe-gate.ps1 生成；性能项按 p95 与基线 ×1.2 或探针预算判定'
        updated   = $timestamp
        repeat    = $Repeat
        machine   = $machine
        entries   = $ordered
    }
    $dir = Split-Path -Parent $baselinePath
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    [IO.File]::WriteAllText($baselinePath, ($payload | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    Write-Host "基线已写入：$baselinePath（$($baselineUpdates.Count) 项）"
}

# ---------- 3/3 归档 ----------
$passCount = @($results | Where-Object { $_.Status -eq 'PASS' }).Count
$failCount = @($results | Where-Object { $_.Status -eq 'FAIL' }).Count
$skipCount = @($results | Where-Object { $_.Status -eq 'SKIP' }).Count
$totalDuration = ($results | Measure-Object -Property DurationSec -Sum).Sum
$rustcVersion = (& rustc -V) -join ''
$machineName = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$os = Get-CimInstance Win32_OperatingSystem
$osText = "$($os.Caption) $($os.Version)"

$rows = foreach ($r in $results) {
    $metrics = if ($r.Metrics) { $r.Metrics } else { '—' }
    $reason = if ($r.Reason) { $r.Reason } else { '—' }
    "| $($r.Id) | $($r.Kind) | $($r.Status) | $($r.DurationSec) s | $metrics | $reason |"
}
$failBlocks = @($results | Where-Object { $_.Status -eq 'FAIL' } | ForEach-Object {
        "### FAIL：$($_.Id)`n`n``````text`n$($_.Excerpt)`n``````n"
    }) -join "`n"

$md = @"
# Probe gate 结果（$dateStamp）

> 本文件由 ``tools/verify/probe-gate.ps1`` 自动生成，重复运行整体覆盖。

- 时间：$timestamp
- 机器：$machineName / $osText
- 工具链：$rustcVersion
- 构建：``cargo test --workspace --release --no-run``（耗时 ${buildSec}s）
- 采样：性能项 ``-Repeat $Repeat``；非性能项固定 1 次
- 基线：``docs/qa/perf-baseline.json``（$(if ($baselineExistedAtStart) { '运行前已存在' } else { '运行前不存在' })$(if ($UpdateBaseline) { '；本轮 -UpdateBaseline 重写' } else { '' })）
- 运行命令：``pwsh -NoProfile -File tools/verify/probe-gate.ps1 -Repeat $Repeat$(if ($UpdateBaseline) { ' -UpdateBaseline' } else { '' })``

## 汇总

- 判定行：$($results.Count)（覆盖 17 条原始 ignored 探针：16 条仍为 ``#[ignore]``，``mindmap_viewbox_debug`` 已晋级为常规断言；``pathological_eval_child`` 在门禁中以 loop / microtask 两种模式各判定一次）
- PASS：$passCount / FAIL：$failCount / SKIP：$skipCount
- 基线写入：$($baselineUpdates.Count) 项（首次运行或 ``-UpdateBaseline`` 时写入）
- 探针总耗时：$([math]::Round($totalDuration, 1)) s
- 结果：$(if ($failCount -eq 0) { '**PASS**（无 FAIL）' } else { '**FAIL**（见下表与失败日志）' })

## 逐条结果

| 探针 | 类型 | 判定 | 耗时 | 指标（min/p50/p95/max） | 原因 |
|---|---|---|---|---|---|
$($rows -join "`n")

## 判定口径

- 性能项：本轮 p95 > 基线 p95 × 1.2 或 > 探针登记预算 → FAIL。预算只登记探针自身已写死的断言
  （``perf_50mb_long_doc`` 打开 ≤ 5000 ms、击键 p95 ≤ 16 ms），其余以基线回归为准。
- 非性能项：退出码 0 + 关键不变量（``PROBE`` 指标等于/不低于下限、CJK 证据文件存在且非空）。
- SKIP 仅用于环境缺失（缺少系统 CJK 字体、50MB 探针内存分配失败），必须写明原因，绝不记 PASS。
- 每条命令超时 300–900 s（默认 600 s），超时记 FAIL 并保留输出摘要。

## 失败日志摘要
$(if ($failBlocks) { $failBlocks } else { '无。' })
"@

$archiveDir = Split-Path -Parent $archivePath
New-Item -ItemType Directory -Path $archiveDir -Force | Out-Null
[IO.File]::WriteAllText($archivePath, $md, [Text.UTF8Encoding]::new($false))
Write-Host ''
Write-Host "PASS=$passCount FAIL=$failCount SKIP=$skipCount（总耗时 $([math]::Round($totalDuration, 1))s）"
Write-Host "结果归档：$archivePath"
if ($failCount -gt 0) { exit 1 } else { exit 0 }
