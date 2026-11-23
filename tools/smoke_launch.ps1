#requires -Version 5.1
<#
.SYNOPSIS
  零级发布门禁：对 release 产物执行真实启动冒烟。

.DESCRIPTION
  启动指定 exe，等待主窗口出现，验证标题与 stderr 无 softbuffer/panic 痕迹，
  然后请求正常关闭并检查退出码。子进程始终使用脚本创建的临时 VEIL_HOME、
  APPDATA、USERPROFILE 与工作目录，不传 CLI 文档路径。唯一输入文档和恢复态都由
  脚本在隔离目录生成；真实用户配置、会话与用户文档路径不提供给被测进程。

  持久化隔离断言：恢复文件只在 VEIL_HOME 被清理，session.txt 与 config.toml 只在
  VEIL_HOME 产生；隔离 APPDATA、工作目录和文档目录的文件列表、SHA-256、长度及
  LastWriteTimeUtc 均保持不变。哨兵不变证明这些位置未被写入，不是 OS 读取审计；
  读取路径优先级由 veil_home_overrides_config_and_data_paths 纯测试覆盖。

  窗口探测只使用 Win32 EnumWindows + GetWindowText 按标题枚举目标进程的顶层
  窗口，不回退到 Process.MainWindowHandle（Session 0 下该句柄为 0 并不代表
  窗口不存在，非零句柄也不能证明交互桌面可用）。

  脚本同时输出会话与桌面上下文（session / target_session / user_interactive /
  desktop）。Session 0 或 UserInteractive=false 时判定为非交互桌面：只断言
  「进程存活、stderr 无 panic、可通过 WM_CLOSE 正常退出且退出码为 0」，用
  stage=pass-degraded 明确标注；窗口、标题与 1 秒预算不在此环境断言。

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
        private static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")]
        private static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

        // 枚举 pid 的可见顶层窗口并返回第一个带非空标题且匹配 pattern 的窗口；
        // pattern 为空时接受任意非空标题。隐藏的 Default IME 等辅助窗口不算主窗口。
        public static bool FindTopLevelWindow(uint pid, string pattern, out IntPtr handle, out string title)
        {
            IntPtr found = IntPtr.Zero;
            string foundTitle = null;
            Regex re = string.IsNullOrEmpty(pattern) ? null : new Regex(pattern);
            EnumWindows(delegate(IntPtr hWnd, IntPtr lParam)
            {
                uint windowPid;
                GetWindowThreadProcessId(hWnd, out windowPid);
                if (windowPid != pid || !IsWindowVisible(hWnd)) return true;
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

        // 非交互桌面下不要求标题或 MainWindowHandle；只向本脚本启动的 pid 所拥有
        // 的顶层窗口投递 WM_CLOSE。返回成功投递的窗口数，0 表示无法请求正常退出。
        public static int CloseTopLevelWindows(uint pid)
        {
            int requested = 0;
            EnumWindows(delegate(IntPtr hWnd, IntPtr lParam)
            {
                uint windowPid;
                GetWindowThreadProcessId(hWnd, out windowPid);
                if (windowPid == pid && PostMessage(hWnd, 0x0010, IntPtr.Zero, IntPtr.Zero))
                    requested++;
                return true;
            }, IntPtr.Zero);
            return requested;
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

# 只枚举脚本创建的隔离树。文件内容哈希证明字节未改；路径、类型、长度和 UTC
# 写入时间同时入指纹，能发现新增、删除、替换与仅改写时间戳。目录也纳入以发现空目录。
function Get-TreeFingerprint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $root = [System.IO.Path]::GetFullPath($Path)
    $rows = @(Get-ChildItem -LiteralPath $root -Recurse -Force | Sort-Object FullName | ForEach-Object {
        $isDirectory = $_.PSIsContainer
        [pscustomobject][ordered]@{
            path = $_.FullName.Substring($root.Length).TrimStart('\')
            kind = $(if ($isDirectory) { 'directory' } else { 'file' })
            length = $(if ($isDirectory) { 0 } else { $_.Length })
            last_write_utc_ticks = $_.LastWriteTimeUtc.Ticks
            sha256 = $(if ($isDirectory) { '' } else { (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash })
        }
    })
    return ConvertTo-Json -InputObject $rows -Compress -Depth 3
}

function Get-StableTreeFingerprint {
    param([Parameter(Mandatory = $true)][string]$Path, [int]$SettleMs = 1500)

    # NTFS 可能延迟更新目录时间戳：刚写完哨兵文件时立即取指纹会与延迟写竞争。
    # 连续两次读取一致才视为稳定，避免把文件系统抖动误判为隔离失败。
    $previous = Get-TreeFingerprint -Path $Path
    $deadline = (Get-Date).AddMilliseconds($SettleMs)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 150
        $current = Get-TreeFingerprint -Path $Path
        if ($current -ceq $previous) { return $current }
        $previous = $current
    }
    return $previous
}

function Assert-TreeUnchanged {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Before,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $after = Get-TreeFingerprint -Path $Path
    if ($Before -cne $after) {
        throw "$Name changed; before=$Before; after=$after"
    }
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
$smokeHome = Join-Path $tempDir "veil-smoke-$stamp.home"
$smokeAppData = Join-Path $tempDir "veil-smoke-$stamp.appdata"
$smokeAppDataVeil = Join-Path $smokeAppData 'Veil'
$smokeWork = Join-Path $tempDir "veil-smoke-$stamp.work"
$smokeUserProfile = Join-Path $tempDir "veil-smoke-$stamp.profile"
$smokeDocuments = Join-Path $smokeUserProfile 'Documents'
$smokeDocument = Join-Path $smokeDocuments 'veil-isolation-input.md'
New-Item -ItemType Directory -Force -ErrorAction Stop -Path `
    $smokeHome, $smokeAppDataVeil, (Join-Path $smokeAppDataVeil 'snapshots'), `
    $smokeWork, (Join-Path $smokeWork 'Veil'), $smokeDocuments | Out-Null

# 所有哨兵都位于临时树；脚本不枚举或指纹化真实 APPDATA/用户文档。
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$sentinelFiles = @(
    (Join-Path $smokeAppDataVeil 'config.toml'),
    (Join-Path $smokeAppDataVeil 'callouts.json'),
    (Join-Path $smokeAppDataVeil 'recovery.dat'),
    (Join-Path $smokeAppDataVeil 'session.txt'),
    (Join-Path $smokeAppDataVeil 'snapshots\decoy.meta'),
    (Join-Path $smokeWork 'Veil\config.toml'),
    (Join-Path $smokeWork 'decoy.md'),
    (Join-Path $smokeDocuments 'do-not-open.md')
)
foreach ($sentinel in $sentinelFiles) {
    [System.IO.File]::WriteAllText(
        $sentinel,
        "VEIL-SMOKE-SENTINEL $stamp`n",
        $utf8NoBom)
}

# 合法恢复槽指向隔离文档且内容与磁盘一致：产品启动应清槽、登记 session，
# 而不是读取隔离 APPDATA 中的同名哨兵。正常关窗再写窗口配置。
$documentText = "# Veil isolated smoke`n`nOnly this temporary document may be read.`n"
[System.IO.File]::WriteAllText($smokeDocument, $documentText, $utf8NoBom)
$documentPathBytes = $utf8NoBom.GetBytes($smokeDocument)
$recoveryText = "VEIL-RECOVERY-1`nbom=0`npath_len=$($documentPathBytes.Length)`n$smokeDocument`n$documentText"
$smokeRecovery = Join-Path $smokeHome 'recovery.dat'
[System.IO.File]::WriteAllText($smokeRecovery, $recoveryText, $utf8NoBom)

$appDataBefore = Get-StableTreeFingerprint -Path $smokeAppData
$workBefore = Get-StableTreeFingerprint -Path $smokeWork
$documentsBefore = Get-StableTreeFingerprint -Path $smokeDocuments
$previousVeilHome = $env:VEIL_HOME
$previousAppData = $env:APPDATA
$previousUserProfile = $env:USERPROFILE
$previousHome = $env:HOME
$previousXdgConfigHome = $env:XDG_CONFIG_HOME
$env:VEIL_HOME = $smokeHome
$env:APPDATA = $smokeAppData
$env:USERPROFILE = $smokeUserProfile
$env:HOME = $smokeUserProfile
$env:XDG_CONFIG_HOME = Join-Path $smokeAppData 'xdg'

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
    $proc = Start-Process -FilePath $exeFull -PassThru -WorkingDirectory $smokeWork `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    try { $targetSessionId = (Get-Process -Id $proc.Id).SessionId } catch { $targetSessionId = -1 }

    $stage = 'window-wait'
    while ($stopwatch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if ($proc.HasExited) { break }

        $match = Find-ProcessWindow -Process $proc -Pattern $ExpectedTitlePattern
        if ($match) {
            $hwnd = $match.Handle
            $windowTitle = $match.Title
            $probeSource = 'enum'
            $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
            break
        }
        Start-Sleep -Milliseconds 50
    }

    # 等待结束后才捕获任意可见标题用于 title-mismatch 诊断，避免启动早期的
    # 辅助窗口或暂时错误的标题被过早当成最终主窗口。
    if ($hwnd -eq [IntPtr]::Zero -and $interactiveDesktop -and $ExpectedTitlePattern -and -not $proc.HasExited) {
        $match = Find-ProcessWindow -Process $proc -Pattern $null
        if ($match) {
            $hwnd = $match.Handle
            $windowTitle = $match.Title
            $probeSource = 'enum-any-title'
            $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
        }
    }

    if ($hwnd -eq [IntPtr]::Zero) {
        $launchMs = [int]$stopwatch.Elapsed.TotalMilliseconds
    }

    if ($proc.HasExited) {
        $proc.WaitForExit()
        $exitCode = $proc.ExitCode
        $stage = 'process-exited'
        $detail = if ($interactiveDesktop) {
            'process exited before a main window appeared'
        } else {
            'process exited during the non-interactive launch/settle interval'
        }
    }
    elseif (-not $interactiveDesktop) {
        # 非交互桌面只断言存活、无 panic、可正常退出；窗口/标题/启动预算均不断言。
        # 必须成功投递 WM_CLOSE 并以退出码 0 结束，强制清理只能用于失败后的回收。
        $stage = 'degraded-alive'
        Start-Sleep -Milliseconds 200
        if ($proc.HasExited) {
            $proc.WaitForExit()
            $exitCode = $proc.ExitCode
            $stage = 'process-exited'
            $detail = 'process exited in a non-interactive session before the settle window elapsed'
        }
        else {
            $stage = 'degraded-close'
            $closeTimeoutMs = [int]([Math]::Max(3, $TimeoutSeconds) * 1000)
            $closeRequests = 0
            try { $closeRequests = [VeilSmoke.Win32Probe]::CloseTopLevelWindows([uint32]$proc.Id) } catch { $closeRequests = 0 }
            if ($closeRequests -le 0) {
                $stage = 'close-request-failed'
                $detail = 'non-interactive process has no top-level window that accepts WM_CLOSE; graceful exit cannot be asserted'
            }
            else {
                $closed = $false
                try { $closed = $proc.WaitForExit($closeTimeoutMs) } catch { $closed = $false }
                if (-not $closed) {
                    $stage = 'close-timeout'
                    $detail = "process did not exit after $closeRequests WM_CLOSE request(s) in a non-interactive session"
                }
                else {
                    $exitCode = $proc.ExitCode
                    if ($exitCode -ne 0) {
                        $stage = 'exit-code'
                        $detail = "graceful close returned exit code $exitCode in a non-interactive session"
                    }
                    else {
                        $stage = 'pass-degraded'
                        $detail = 'non-interactive desktop; asserted=process-alive,no-stderr-panic,graceful-exit-0; not_asserted=window,title,launch-budget'
                    }
                }
            }
        }
    }
    elseif ($hwnd -eq [IntPtr]::Zero) {
        $stage = 'window-timeout'
        $detail = "no top-level window with a title within ${TimeoutSeconds}s (probe=enum-title)"
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
            try { $closeRequested = [VeilSmoke.Win32Probe]::CloseWindow($hwnd) } catch { $closeRequested = $false }
            if (-not $closeRequested) {
                try { $closeRequested = [VeilSmoke.Win32Probe]::CloseTopLevelWindows([uint32]$proc.Id) -gt 0 } catch { $closeRequested = $false }
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
                    $detail = 'window did not close after WM_CLOSE'
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
    $env:VEIL_HOME = $previousVeilHome
    $env:APPDATA = $previousAppData
    $env:USERPROFILE = $previousUserProfile
    $env:HOME = $previousHome
    $env:XDG_CONFIG_HOME = $previousXdgConfigHome
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
    $detail = 'stderr contains a softbuffer/panic signature after otherwise successful smoke assertions'
}

$isolationText = 'not-run'
if ($stage -in @('pass', 'pass-degraded')) {
    try {
        if (Test-Path -LiteralPath $smokeRecovery) {
            throw 'recovery.dat was not cleared inside VEIL_HOME'
        }
        $configArtifact = Join-Path $smokeHome 'config.toml'
        $sessionArtifact = Join-Path $smokeHome 'session.txt'
        if (-not (Test-Path -LiteralPath $configArtifact -PathType Leaf) -or
            (Get-Item -LiteralPath $configArtifact).Length -le 0) {
            throw 'config.toml was not produced inside VEIL_HOME'
        }
        if (-not (Test-Path -LiteralPath $sessionArtifact -PathType Leaf)) {
            throw 'session.txt was not produced inside VEIL_HOME'
        }
        $sessionText = [System.IO.File]::ReadAllText($sessionArtifact)
        if (-not $sessionText.Contains($smokeDocument)) {
            throw "session.txt does not reference only the isolated document: $sessionText"
        }
        Assert-TreeUnchanged -Name 'isolated APPDATA sentinel tree' -Before $appDataBefore -Path $smokeAppData
        Assert-TreeUnchanged -Name 'isolated working-directory sentinel tree' -Before $workBefore -Path $smokeWork
        Assert-TreeUnchanged -Name 'isolated documents tree' -Before $documentsBefore -Path $smokeDocuments
        $isolationText = 'pass'
    }
    catch {
        $stage = 'isolation-failed'
        $detail = $_.Exception.Message
        $isolationText = 'fail'
    }
}

$status = if ($stage -in @('pass', 'pass-degraded')) { 'PASS' } else { 'FAIL' }
$hwndText = if ($hwnd -ne [IntPtr]::Zero) { '0x{0:X}' -f $hwnd.ToInt64() } else { 'none' }
$exitText = if ($null -ne $exitCode) {
    $exitUnsigned = [uint32]([int64]$exitCode -band 4294967295)
    ('{0} (0x{1:X8})' -f $exitCode, $exitUnsigned)
} else { 'n/a' }
$titleText = if ($windowTitle) { '"' + $windowTitle.Replace('"', "'") + '"' } else { 'none' }
$panicText = if ($panicHits.Count -gt 0) { 'true' } else { 'false' }

Write-Host ("SMOKE {0} exe={1} stage={2} launch_ms={3} max_ms={4} hwnd={5} title={6} exit_code={7} panic={8} timeout_s={9} session={10} target_session={11} user_interactive={12} desktop={13} probe={14} veil_home={15}" -f `
    $status, $exeFull, $stage, $launchMs, $MaxLaunchMs, $hwndText, $titleText, $exitText, $panicText, $TimeoutSeconds, `
    $currentSessionId, $targetSessionId, $userInteractive.ToString().ToLowerInvariant(), $interactiveDesktop.ToString().ToLowerInvariant(), $probeSource, 'isolated')

if ($isolationText -eq 'pass') {
    Write-Host 'SMOKE ISOLATION PASS config=veil_home session=veil_home recovery=cleared_in_veil_home appdata_sentinels=unchanged workdir_sentinels=unchanged documents=unchanged fingerprint=path+kind+length+last_write_utc+sha256 read_audit=not-covered'
}

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
Remove-Item -LiteralPath $smokeHome, $smokeAppData, $smokeWork, $smokeUserProfile -Recurse -Force -ErrorAction SilentlyContinue

if ($status -eq 'FAIL') { exit 1 }
exit 0
