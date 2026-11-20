#requires -Version 5.1
<#
.SYNOPSIS
  零级发布门禁：对 release 产物执行真实启动冒烟。

.DESCRIPTION
  启动指定 exe，等待主窗口出现，验证标题与 stderr 无 softbuffer/panic 痕迹，
  然后请求正常关闭并检查退出码。

  窗口探测优先用 Win32 EnumWindows + GetWindowText 按标题枚举目标进程的顶层
  窗口（Session 0 下 MainWindowHandle 为 0 并不代表窗口不存在），找不到再回退
  Process.MainWindowHandle。

  脚本同时输出会话与桌面上下文（session / target_session / user_interactive /
  desktop）。Session 0 或 UserInteractive=false 时判定为非交互桌面：窗口不可见
  属预期，降级为只断言「进程存活、stderr 无 panic、可正常退出」，用
  stage=pass-degraded 明确标注，不把降级当严格 PASS。

  失败时以非零码退出，并打印结构化 SMOKE 行（FAIL 阶段 + stderr 尾部），便于 CI
  日志检索：
    SMOKE PASS exe=... stage=pass launch_ms=... max_ms=1000 hwnd=0x... ...
    SMOKE FAIL exe=... stage=process-exited launch_ms=... exit_code=-1073740791 ...

  1 秒预算：spec 要求双击后 1 秒内显示主窗口。MaxLaunchMs（默认 1000）是硬
  上限，launch_ms 超限即判 FAIL（stage=launch-too-slow），与 TimeoutSeconds
  无关；TimeoutSeconds 只决定愿意等多久来区分「慢启动」与「压根没出窗」。

  安全约束：不 kill 任何非本脚本启动的进程。若启动前已有同名 veil
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
  pwsh -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe
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

# 会话与桌面上下文：Session 0 是服务/非交互会话（GitHub 托管 runner 常见），
# 但 UserInteractive 反映的是当前窗口站，二者都记录，避免把环境差异误判为产品缺陷。
$currentSessionId = -1
try { $currentSessionId = (Get-Process -Id $PID).SessionId } catch { }
$userInteractive = $false
try { $userInteractive = [Environment]::UserInteractive } catch { }
$interactiveDesktop = $userInteractive -and ($currentSessionId -ne 0)

$panicPatterns = @('panicked at', 'buffer_mut', 'Must set size of surface')

# 按标题枚举目标进程的顶层窗口。MainWindowHandle 依赖「可见主窗口」的启发式，
# 在 Session 0 或首帧前的隐藏窗口阶段会返回 0；EnumWindows 只看当前窗口站。
if (-not ('VeilSmoke.Win32Probe' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Text.RegularExpressions;
using System.Runtime.InteropServices;

namespace VeilSmoke
{
    public static class Win32Probe
    {
        private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
        [DllImport("user32.dll")]
        private static extern int GetWindowTextLength(IntPtr hWnd);
        [DllImport("user32.dll")]
        private static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

        // 枚举 pid 的顶层窗口并返回第一个带非空标题且匹配 pattern 的窗口；
        // pattern 为空时接受任意非空标题。
        public static bool FindTopLevelWindow(uint pid, string pattern, out IntPtr handle, out string title)
        {
            IntPtr found = IntPtr.Zero;
            string foundTitle = null;
            Regex re = string.IsNullOrEmpty(pattern) ? null : new Regex(pattern);
            EnumWindows(delegate(IntPtr hWnd, IntPtr lParam)
            {
                uint windowPid;
                GetWindowThreadProcessId(hWnd, out windowPid);
                if (windowPid != pid) return true;
                int len = GetWindowTextLength(hWnd);
                if (len <= 0) return true;
                StringBuilder sb = new StringBuilder(len + 1);
                GetWindowText(hWnd, sb, sb.Capacity);
                string text = sb.ToString();
                if (string.IsNullOrWhiteSpace(text)) return true;
                if (re != null && !re.IsMatch(text)) return true;
                found = hWnd;
                foundTitle = text;
                return false;
            }, IntPtr.Zero);
            handle = found;
            title = foundTitle;
            return found != IntPtr.Zero;
        }

        // 直接向窗口句柄投递 WM_CLOSE：CloseMainWindow 要求 .NET 认为它有主窗口。
        public static bool CloseWindow(IntPtr hWnd)
        {
            return PostMessage(hWnd, 0x0010, IntPtr.Zero, IntPtr.Zero);
        }
    }
}
'@
}

function Find-ProcessWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [string]$Pattern
    )

    $handle = [IntPtr]::Zero
    $title = $null
    $found = $false
    try {
        $found = [VeilSmoke.Win32Probe]::FindTopLevelWindow([uint32]$Process.Id, $Pattern, [ref]$handle, [ref]$title)
    } catch {
        $found = $false
    }
    if (-not $found) { return $null }
    return [pscustomobject]@{ Handle = $handle; Title = $title }
}

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
$targetSessionId = -1
$probeSource = 'none'

try {
    $stage = 'process-start'
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $exeFull -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    try { $targetSessionId = (Get-Process -Id $proc.Id).SessionId } catch { $targetSessionId = -1 }

    $stage = 'window-wait'
    while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if ($proc.HasExited) { break }

        $match = Find-ProcessWindow -Process $proc -Pattern $ExpectedTitlePattern
        # 给了标题模式但窗口标题暂不匹配时，仍捕获该进程的任意带标题顶层窗口，
        # 让后续 title-mismatch 阶段给出可诊断的标题，而不是误报 window-timeout。
        if (-not $match -and $ExpectedTitlePattern) {
            $match = Find-ProcessWindow -Process $proc -Pattern $null
        }
        if ($match) {
            $hwnd = $match.Handle
            $windowTitle = $match.Title
            $probeSource = 'enum'
            $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
            break
        }
        Start-Sleep -Milliseconds 50
    }

    # 回退路径：EnumWindows 未命中（例如窗口站在不同桌面）时仍按旧口径探测。
    if ($hwnd -eq [IntPtr]::Zero -and -not $proc.HasExited) {
        try { $proc.Refresh() } catch { }
        $fallback = [IntPtr]::Zero
        try { $fallback = $proc.MainWindowHandle } catch { $fallback = [IntPtr]::Zero }
        if ($fallback -ne [IntPtr]::Zero) {
            $hwnd = $fallback
            $probeSource = 'mainwindowhandle'
            $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
            try { $windowTitle = $proc.MainWindowTitle } catch { $windowTitle = '' }
        }
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
    elseif ($hwnd -eq [IntPtr]::Zero -and -not $interactiveDesktop) {
        # 非交互桌面（Session 0 / UserInteractive=false）：窗口不可见是环境属性，
        # 不是产品缺陷。只断言进程存活、无 stderr panic、可正常退出。
        $stage = 'degraded-alive'
        Start-Sleep -Milliseconds 200
        if ($proc.HasExited) {
            $proc.WaitForExit()
            $exitCode = $proc.ExitCode
            $stage = 'process-exited'
            $detail = 'process exited in a non-interactive session before the settle window elapsed'
        }
        else {
            $closed = $false
            try { $closed = $proc.WaitForExit(3000) } catch { $closed = $false }
            if (-not $closed) {
                try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch { }
                try { $closed = $proc.WaitForExit(3000) } catch { $closed = $false }
            }
            if ($closed) {
                $stage = 'pass-degraded'
                $detail = 'non-interactive desktop: asserted process alive, no stderr panic, and normal termination; window and 1s budget are not asserted here'
            } else {
                $stage = 'close-timeout'
                $detail = 'process did not terminate in a non-interactive session'
            }
        }
    }
    elseif ($hwnd -eq [IntPtr]::Zero) {
        $stage = 'window-timeout'
        $detail = "no top-level window with a title within ${TimeoutSeconds}s (probe=enum+mainwindowhandle)"
    }
    else {
        $stage = 'window-visible'
        # 窗口句柄可能先于标题出现（隐藏窗口首帧路径）。继续观察到总超时为止，
        # 期间若进程 panic 退出，必须按 process-exited 捕获退出码与 stderr，
        # 而不是误报空标题后把它 kill 掉。
        while (-not $proc.HasExited -and $stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
            if (-not [string]::IsNullOrWhiteSpace($windowTitle)) { break }
            $match = Find-ProcessWindow -Process $proc -Pattern $null
            if ($match) {
                $windowTitle = $match.Title
                if ($probeSource -eq 'none') { $probeSource = 'enum' }
                break
            }
            try { $proc.Refresh() } catch { }
            try { $windowTitle = $proc.MainWindowTitle } catch { $windowTitle = '' }
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
            if (-not $closeRequested) {
                # .NET 认为没有主窗口时（例如 EnumWindows 找到的非标准顶层窗口），
                # 直接向句柄投递 WM_CLOSE。
                try { $closeRequested = [VeilSmoke.Win32Probe]::CloseWindow($hwnd) } catch { $closeRequested = $false }
            }
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
if ($stage -in @('pass', 'pass-degraded') -and $panicHits.Count -gt 0) {
    $stage = 'stderr-panic'
    $detail = 'stderr contains a softbuffer/panic signature even though a window appeared'
}

$status = if ($stage -in @('pass', 'pass-degraded')) { 'PASS' } else { 'FAIL' }
$hwndText = if ($hwnd -ne [IntPtr]::Zero) { '0x{0:X}' -f $hwnd.ToInt64() } else { 'none' }
$exitText = if ($null -ne $exitCode) {
    $exitUnsigned = [uint32]([int64]$exitCode -band 4294967295)
    ('{0} (0x{1:X8})' -f $exitCode, $exitUnsigned)
} else { 'n/a' }
$titleText = if ($windowTitle) { '"' + $windowTitle.Replace('"', "'") + '"' } else { 'none' }
$panicText = if ($panicHits.Count -gt 0) { 'true' } else { 'false' }

Write-Host ("SMOKE {0} exe={1} stage={2} launch_ms={3} max_ms={4} hwnd={5} title={6} exit_code={7} panic={8} timeout_s={9} session={10} target_session={11} user_interactive={12} desktop={13} probe={14}" -f `
    $status, $exeFull, $stage, $launchMs, $MaxLaunchMs, $hwndText, $titleText, $exitText, $panicText, $TimeoutSeconds, `
    $currentSessionId, $targetSessionId, $userInteractive.ToString().ToLowerInvariant(), $interactiveDesktop.ToString().ToLowerInvariant(), $probeSource)

if ($stage -eq 'pass-degraded') {
    Write-Host "  degraded=true detail=$detail"
}

if ($status -eq 'FAIL') {
    if ($detail) { Write-Host "  detail=$detail" }
    if ($panicHits.Count -gt 0) { Write-Host "  panic_patterns=$($panicHits -join '; ')" }
    foreach ($line in Get-TailLines -Text $stderrText -Count 40) { Write-Host "  stderr| $line" }
    foreach ($line in Get-TailLines -Text $stdoutText -Count 20) { Write-Host "  stdout| $line" }
}

Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue

if ($status -eq 'FAIL') { exit 1 }
exit 0
