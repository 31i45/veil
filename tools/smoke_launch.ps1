#requires -Version 5.1
<#
.SYNOPSIS
  零级发布门禁：对 release 产物执行真实启动冒烟。

.DESCRIPTION
  启动指定 exe，等待主窗口出现（MainWindowHandle 非零），验证标题与 stderr 无
  softbuffer/panic 痕迹，然后请求正常关闭并检查退出码。

  失败时以非零码退出，并打印结构化 SMOKE 行（FAIL 阶段 + stderr 尾部），便于 CI
  日志检索：
    SMOKE PASS exe=... stage=pass launch_ms=... max_ms=1000 hwnd=0x... ...
    SMOKE FAIL exe=... stage=process-exited launch_ms=... exit_code=-1073740791 ...

  1 秒预算：spec 要求双击后 1 秒内显示主窗口。MaxLaunchMs（默认 1000）是硬
  上限，launch_ms 超限即判 FAIL（stage=launch-too-slow），与 TimeoutSeconds
  无关；TimeoutSeconds 只决定愿意等多久来区分「慢启动」与「压根没出窗」。

  安全约束：不 kill 任何非本脚本启动的进程。若启动前已有同名 veil/veil-full
  进程，直接失败并要求用户先手动关闭。

.PARAMETER ExePath
  被测可执行文件路径（必填）。

.PARAMETER TimeoutSeconds
  等待主窗口出现的超时秒数，默认 5。只控制探测时长，不放宽启动预算。

.PARAMETER MaxLaunchMs
  主窗口出现耗时上限（毫秒），默认 1000。launch_ms 超过该值即判 FAIL
  （stage=launch-too-slow），即使窗口最终出现、标题正确、退出码正常。

.PARAMETER ExpectedTitlePattern
  可选：主窗口标题必须匹配的正则表达式。

.EXAMPLE
  pwsh -File tools/smoke_launch.ps1 -ExePath target-default/release/veil.exe

.EXAMPLE
  pwsh -File tools/smoke_launch.ps1 -ExePath target-full/release/veil-full.exe -TimeoutSeconds 10
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$ExePath,

    [ValidateRange(1, 600)]
    [int]$TimeoutSeconds = 5,

    [ValidateRange(1, 600000)]
    [int]$MaxLaunchMs = 1000,

    [string]$ExpectedTitlePattern
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$panicPatterns = @('panicked at', 'buffer_mut', 'Must set size of surface')

function Get-LogText {
    param([string]$Path)

    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return '' }
    # 进程刚退出时重定向文件可能仍被短暂占用，重试读取。
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            $stream = [System.IO.File]::Open(
                $Path,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::Read,
                [System.IO.FileShare]::ReadWrite)
            try {
                $reader = New-Object System.IO.StreamReader($stream)
                try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
            } finally { $stream.Dispose() }
        } catch {
            Start-Sleep -Milliseconds 50
        }
    }
    return ''
}

function Get-TailLines {
    param([string]$Text, [int]$Count = 40)

    if (-not $Text) { return @() }
    $lines = $Text -split "`r?`n"
    if ($lines.Count -le $Count) { return $lines }
    return $lines[($lines.Count - $Count)..($lines.Count - 1)]
}

$resolved = $null
try { $resolved = (Resolve-Path -LiteralPath $ExePath -ErrorAction Stop).ProviderPath } catch { }
if (-not $resolved -or -not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
    Write-Host "SMOKE FAIL exe=$ExePath stage=exe-missing timeout_s=$TimeoutSeconds"
    Write-Host '  detail=executable not found; build the release binary first'
    exit 1
}
$exeFull = $resolved
$exeName = [System.IO.Path]::GetFileNameWithoutExtension($exeFull)

# 启动前拒绝同名进程：绝不杀用户已在运行的实例。
$existing = @(Get-Process -Name $exeName -ErrorAction SilentlyContinue)
if ($existing.Count -gt 0) {
    $pidList = ($existing | ForEach-Object { $_.Id }) -join ','
    Write-Host "SMOKE FAIL exe=$exeFull stage=process-already-running"
    Write-Host "  detail=found running $exeName process(es) pid=$pidList; close them manually and rerun (this script never kills pre-existing processes)"
    exit 1
}

$stamp = [guid]::NewGuid().ToString('N')
$tempDir = [System.IO.Path]::GetTempPath()
$stdoutPath = Join-Path $tempDir "veil-smoke-$stamp.stdout.log"
$stderrPath = Join-Path $tempDir "veil-smoke-$stamp.stderr.log"

$proc = $null
$stage = 'init'
$detail = ''
$launchMs = 0
$hwnd = [IntPtr]::Zero
$windowTitle = ''
$exitCode = $null
$stderrText = ''
$stdoutText = ''

try {
    $stage = 'process-start'
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $exeFull -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath

    $stage = 'window-wait'
    while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if ($proc.HasExited) { break }

        try { $proc.Refresh() } catch { }
        $handle = [IntPtr]::Zero
        try { $handle = $proc.MainWindowHandle } catch { $handle = [IntPtr]::Zero }
        if ($handle -ne [IntPtr]::Zero) {
            $hwnd = $handle
            $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
            break
        }
        Start-Sleep -Milliseconds 50
    }
    if ($hwnd -eq [IntPtr]::Zero) {
        $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
    }

    if ($proc.HasExited) {
        $proc.WaitForExit()
        $exitCode = $proc.ExitCode
        $stage = 'process-exited'
        $detail = 'process exited before a main window appeared'
    }
    elseif ($hwnd -eq [IntPtr]::Zero) {
        $stage = 'window-timeout'
        $detail = "no main window within ${TimeoutSeconds}s"
    }
    else {
        $stage = 'window-visible'
        # 窗口句柄可能先于标题出现（隐藏窗口首帧路径）。继续观察到总超时为止，
        # 期间若进程 panic 退出，必须按 process-exited 捕获退出码与 stderr，
        # 而不是误报空标题后把它 kill 掉。
        while (-not $proc.HasExited -and $stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
            try { $proc.Refresh() } catch { }
            try { $windowTitle = $proc.MainWindowTitle } catch { $windowTitle = '' }
            if (-not [string]::IsNullOrWhiteSpace($windowTitle)) { break }
            Start-Sleep -Milliseconds 50
        }

        if ($proc.HasExited) {
            $proc.WaitForExit()
            $exitCode = $proc.ExitCode
            $stage = 'process-exited'
            $detail = 'process exited before its main window title was set'
        }
        elseif ([string]::IsNullOrWhiteSpace($windowTitle)) {
            $stage = 'window-title-empty'
            $detail = 'main window has an empty title'
        }
        elseif ($ExpectedTitlePattern -and $windowTitle -notmatch $ExpectedTitlePattern) {
            $stage = 'title-mismatch'
            $detail = "title '$windowTitle' does not match pattern '$ExpectedTitlePattern'"
        }
        else {
            $stage = 'close'
            $closeTimeoutMs = [int]([Math]::Max(3, $TimeoutSeconds) * 1000)
            $closeRequested = $false
            try { $closeRequested = $proc.CloseMainWindow() } catch { $closeRequested = $false }
            $closed = $false
            if ($closeRequested) { $closed = $proc.WaitForExit($closeTimeoutMs) }

            if (-not $closed) {
                if ($proc.HasExited) {
                    $exitCode = $proc.ExitCode
                    $stage = 'process-exited'
                    $detail = 'process exited while handling the close request'
                } else {
                    $stage = 'close-timeout'
                    $detail = 'window did not close after CloseMainWindow'
                }
            }
            else {
                $exitCode = $proc.ExitCode
                if ($exitCode -ne 0) {
                    $stage = 'exit-code'
                    $detail = "graceful close returned exit code $exitCode"
                }
                elseif ($launchMs -gt $MaxLaunchMs) {
                    # 窗口最终出现且可正常关闭，但已超出 1 秒预算：按 FAIL 处理，
                    # 不能因为 TimeoutSeconds（默认 5s）没等到超时就算 PASS。
                    $stage = 'launch-too-slow'
                    $detail = "main window appeared after ${launchMs} ms, exceeding the ${MaxLaunchMs} ms budget"
                }
                else {
                    $stage = 'pass'
                }
            }
        }
    }
}
catch {
    $stage = 'error'
    $detail = $_.Exception.Message
}
finally {
    # 只回收本脚本启动的 PID；失败路径也必须清理，避免留下无窗口进程。
    if ($proc) {
        try {
            if (-not $proc.HasExited) {
                Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
                $proc.WaitForExit(2000) | Out-Null
            }
        } catch { }
    }
    Start-Sleep -Milliseconds 100
    $stderrText = Get-LogText -Path $stderrPath
    $stdoutText = Get-LogText -Path $stdoutPath
}

$panicHits = @()
foreach ($pattern in $panicPatterns) {
    if ($stderrText -and $stderrText.IndexOf($pattern, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        $panicHits += $pattern
    }
}
if ($stage -eq 'pass' -and $panicHits.Count -gt 0) {
    $stage = 'stderr-panic'
    $detail = 'stderr contains a softbuffer/panic signature even though a window appeared'
}

$status = if ($stage -eq 'pass') { 'PASS' } else { 'FAIL' }
$hwndText = if ($hwnd -ne [IntPtr]::Zero) { '0x{0:X}' -f $hwnd.ToInt64() } else { 'none' }
$exitText = if ($null -ne $exitCode) {
    $exitUnsigned = [uint32]([int64]$exitCode -band 4294967295)
    ('{0} (0x{1:X8})' -f $exitCode, $exitUnsigned)
} else { 'n/a' }
$titleText = if ($windowTitle) { '"' + $windowTitle.Replace('"', "'") + '"' } else { 'none' }
$panicText = if ($panicHits.Count -gt 0) { 'true' } else { 'false' }

Write-Host ("SMOKE {0} exe={1} stage={2} launch_ms={3} max_ms={4} hwnd={5} title={6} exit_code={7} panic={8} timeout_s={9}" -f `
    $status, $exeFull, $stage, $launchMs, $MaxLaunchMs, $hwndText, $titleText, $exitText, $panicText, $TimeoutSeconds)

if ($status -eq 'FAIL') {
    if ($detail) { Write-Host "  detail=$detail" }
    if ($panicHits.Count -gt 0) { Write-Host "  panic_patterns=$($panicHits -join '; ')" }
    foreach ($line in Get-TailLines -Text $stderrText -Count 40) { Write-Host "  stderr| $line" }
    foreach ($line in Get-TailLines -Text $stdoutText -Count 20) { Write-Host "  stdout| $line" }
}

Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue

if ($status -eq 'FAIL') { exit 1 }
exit 0
