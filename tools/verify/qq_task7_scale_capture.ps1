# qq-task7 缩放抽样取证：在同一个 release 进程里用缩放快捷键（Ctrl+=）切到
# 100% / 1.2×（125% 槽位，见下）/ 150% / 200%，各截一张整窗图。
#
# 诚实标注（重要）：
#   - 这是「等效缩放抽样（user_zoom 路径）」，不是系统 DPI 切换；effective scale
#     在源码里是 dpi_scale × user_zoom，故两者共用同一条几何路径。
#   - 本版产品把 user_zoom 按 0.1 量化（crates/veil-ui/src/interaction.rs），
#     快捷键无法得到 1.25；scale-125.png 的实际缩放是 120%，文件名沿用任务槽位。
#     证据日志逐条记录「槽位 / 实际 zoom」。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/qq_task7_scale_capture.ps1
#
# 失败（找不到 exe / 窗口超时 / 置前失败 / 截图全黑）一律 exit 1，不留下冒充证据的空白图。

[CmdletBinding()]
param(
    [string]$Exe = '',
    [string]$Doc = '',
    [string]$OutDir = '',
    [int]$SettleMs = 900
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'evidence\verify\qq-task7' }

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$script:logPath = Join-Path $OutDir 'scale-capture.log'

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    Add-Content -LiteralPath $script:logPath -Value $line -Encoding utf8
    Write-Host $line
}

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
if (-not ('QQTask7.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace QQTask7 {
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public static class Win32 {
        public delegate bool EnumProc(IntPtr hWnd, IntPtr param);

        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr param);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT value, int size);
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    }

    public static class K {
        public const uint PW_RENDERFULLCONTENT = 2;
        public const uint SWP_NOSIZE = 0x0001;
        public const uint SWP_NOMOVE = 0x0002;
        public const uint SWP_NOACTIVATE = 0x0010;
        public const int SW_RESTORE = 9;
        public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
        public static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    }
}
'@
}

[QQTask7.Win32]::SetProcessDPIAware() | Out-Null

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [QQTask7.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [QQTask7.Win32]::IsWindowVisible($h)) { return $true }
        if ([QQTask7.Win32]::IsIconic($h)) { return $true }
        $r = New-Object QQTask7.RECT
        [QQTask7.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [QQTask7.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [QQTask7.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
    return @($script:enumWindows | Sort-Object { $_.Hwnd.ToInt64() })
}

function Wait-MainWindow {
    param($Process, [int]$TimeoutMs)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        if ($Process.HasExited) {
            throw "进程在窗口出现前退出：PID=$($Process.Id) ExitCode=$($Process.ExitCode)"
        }
        $windows = @(Get-ProcessWindows -ProcessId $Process.Id)
        if ($windows.Count -gt 0) { return $windows[0] }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    throw "等待主窗口超时（${TimeoutMs}ms）"
}

function Ensure-Foreground {
    param([IntPtr]$Hwnd)
    [QQTask7.Win32]::ShowWindow($Hwnd, [QQTask7.K]::SW_RESTORE) | Out-Null
    for ($i = 0; $i -lt 5; $i++) {
        [QQTask7.Win32]::SetForegroundWindow($Hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        if ([QQTask7.Win32]::GetForegroundWindow() -eq $Hwnd) { return }
    }
    throw "SetForegroundWindow 失败：无法把 Veil 置前，拒绝发送缩放快捷键"
}

function Send-ZoomIn {
    param([IntPtr]$Hwnd, [int]$Times)
    for ($i = 0; $i -lt $Times; $i++) {
        Ensure-Foreground -Hwnd $Hwnd
        [System.Windows.Forms.SendKeys]::SendWait('^=')
        Start-Sleep -Milliseconds 150
    }
}

function Test-BlankBitmap {
    param([System.Drawing.Bitmap]$Bitmap)
    $colors = @{}
    $stepX = [Math]::Max(1, [int]($Bitmap.Width / 16))
    $stepY = [Math]::Max(1, [int]($Bitmap.Height / 10))
    for ($y = 2; $y -lt $Bitmap.Height; $y += $stepY) {
        for ($x = 2; $x -lt $Bitmap.Width; $x += $stepX) {
            $colors[$Bitmap.GetPixel($x, $y).ToArgb()] = $true
        }
    }
    return ($colors.Count -le 2)
}

function Get-AverageLuminance {
    param([System.Drawing.Bitmap]$Bitmap)
    $sum = 0.0
    $count = 0
    $stepX = [Math]::Max(1, [int]($Bitmap.Width / 24))
    $stepY = [Math]::Max(1, [int]($Bitmap.Height / 16))
    for ($y = 2; $y -lt $Bitmap.Height; $y += $stepY) {
        for ($x = 2; $x -lt $Bitmap.Width; $x += $stepX) {
            $c = $Bitmap.GetPixel($x, $y)
            $sum += 0.2126 * $c.R + 0.7152 * $c.G + 0.0722 * $c.B
            $count++
        }
    }
    if ($count -eq 0) { return -1.0 }
    return $sum / $count
}

function Save-WindowCapture {
    param([IntPtr]$Hwnd, [string]$Path)
    $wr = New-Object QQTask7.RECT
    [QQTask7.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) { throw "窗口尺寸无效 ${w}x${h}" }

    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $printed = $false
    # PrintWindow 在刚完成缩放重绘时偶发全黑：先重试 3 次，仍失败才降级为屏幕捕获，
    # 避免把桌面其它窗口的边缘像素混进证据图。
    for ($attempt = 0; $attempt -lt 3 -and -not $printed; $attempt++) {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        try {
            $printed = [QQTask7.Win32]::PrintWindow($Hwnd, $hdc, [QQTask7.K]::PW_RENDERFULLCONTENT)
        } finally {
            $g.ReleaseHdc($hdc)
            $g.Dispose()
        }
        if ($printed) {
            $blankProbe = Test-BlankBitmap -Bitmap $bmp
            if ($blankProbe) {
                $printed = $false
                Write-Log 'WARN' "PrintWindow 第 $($attempt + 1) 次返回空白，重试"
                Start-Sleep -Milliseconds 350
            }
        }
    }

    $method = 'PrintWindow'
    $degraded = $false
    $blank = if ($printed) { Test-BlankBitmap -Bitmap $bmp } else { $true }
    if (-not $printed -or $blank) {
        # 屏幕捕获只用「可见外框」：GetWindowRect 含 DWM 不可见边框，直接按它复制会把
        # 周围桌面/其它窗口的像素带进证据图。
        $visible = New-Object QQTask7.RECT
        $dwmOk = [QQTask7.Win32]::DwmGetWindowAttribute($Hwnd, 9, [ref]$visible, 16) -eq 0
        if (-not $dwmOk -or ($visible.Right - $visible.Left) -le 0 -or ($visible.Bottom - $visible.Top) -le 0) {
            $visible = $wr
        }
        $vw = $visible.Right - $visible.Left
        $vh = $visible.Bottom - $visible.Top
        $flags = ([QQTask7.K]::SWP_NOMOVE -bor [QQTask7.K]::SWP_NOSIZE -bor [QQTask7.K]::SWP_NOACTIVATE)
        $screen = New-Object System.Drawing.Bitmap($vw, $vh)
        [QQTask7.Win32]::SetWindowPos($Hwnd, [QQTask7.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 300
            $g2 = [System.Drawing.Graphics]::FromImage($screen)
            try {
                $g2.CopyFromScreen($visible.Left, $visible.Top, 0, 0, (New-Object System.Drawing.Size($vw, $vh)))
            } finally {
                $g2.Dispose()
            }
        } finally {
            [QQTask7.Win32]::SetWindowPos($Hwnd, [QQTask7.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        }
        $bmp.Dispose()
        $bmp = $screen
        $method = 'CopyFromScreen'
        $degraded = $true
        $blank = Test-BlankBitmap -Bitmap $bmp
    }

    if ($blank) {
        $blankPath = [IO.Path]::ChangeExtension($Path, '.blank.png')
        $bmp.Save($blankPath, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        throw "截图全黑/纯色，已保留诊断图：$blankPath"
    }
    $luminance = Get-AverageLuminance -Bitmap $bmp
    $actualW = $bmp.Width
    $actualH = $bmp.Height
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $info = Get-Item -LiteralPath $Path
    Write-Log 'PASS' "已保存 $Path（${actualW}x${actualH}，方法=$method，降级=$degraded，平均亮度=$([Math]::Round($luminance,1))，字节=$($info.Length)）"
}

$exitCode = 0
$proc = $null
$tempRoot = ''
$stderrPath = ''
try {
    if (-not (Test-Path -LiteralPath $Exe)) {
        throw "找不到 release 可执行文件：$Exe（请先 cargo build --release -p veil）"
    }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    Write-Log 'INFO' "开始：Exe=$Exe OutDir=$OutDir"

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-qq-task7-{0}-{1}" -f $stamp, $PID)
    $veilHome = Join-Path $tempRoot 'home'
    $appData = Join-Path $tempRoot 'appdata'
    $localAppData = Join-Path $tempRoot 'localappdata'
    $tempDir = Join-Path $tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null

    if (-not $Doc) {
        $readme = Join-Path $repoRoot 'README.zh-CN.md'
        if (-not (Test-Path -LiteralPath $readme)) { throw "缺少 README.zh-CN.md：$readme" }
        $Doc = Join-Path $tempRoot 'qq-task7-doc.md'
        Copy-Item -LiteralPath $readme -Destination $Doc -Force
    }
    if (-not (Test-Path -LiteralPath $Doc)) { throw "找不到 -Doc 文件：$Doc" }

    # 浅色观感对比：固定 light 主题，排除主题变量对缩放的干扰。
    [IO.File]::WriteAllText((Join-Path $veilHome 'config.toml'), "mode = `"light`"`n", [Text.UTF8Encoding]::new($false))
    $stderrPath = Join-Path $tempRoot 'stderr.log'
    $proc = Start-Process -FilePath $Exe -ArgumentList @($Doc) -PassThru `
        -RedirectStandardOutput (Join-Path $tempRoot 'stdout.log') -RedirectStandardError $stderrPath `
        -Environment @{
            VEIL_HOME = $veilHome
            APPDATA = $appData
            LOCALAPPDATA = $localAppData
            TEMP = $tempDir
            TMP = $tempDir
        }
    Write-Log 'INFO' "已启动 release：PID=$($proc.Id) 文档=$Doc"

    $window = Wait-MainWindow -Process $proc -TimeoutMs 25000
    $hwnd = [IntPtr]$window.Hwnd
    Write-Log 'INFO' "主窗口：HWND=$hwnd 标题=$($window.Title)"
    Start-Sleep -Milliseconds 1200
    Ensure-Foreground -Hwnd $hwnd

    # 槽位 → (实际 user_zoom, 需要追加的 Ctrl+= 次数)
    $plan = @(
        @{ Slot = 100; File = 'scale-100.png'; Zoom = 1.0; Presses = 0 },
        @{ Slot = 125; File = 'scale-125.png'; Zoom = 1.2; Presses = 2 },
        @{ Slot = 150; File = 'scale-150.png'; Zoom = 1.5; Presses = 3 },
        @{ Slot = 200; File = 'scale-200.png'; Zoom = 2.0; Presses = 5 }
    )

    foreach ($step in $plan) {
        if ($step.Presses -gt 0) {
            Send-ZoomIn -Hwnd $hwnd -Times $step.Presses
        }
        Start-Sleep -Milliseconds $SettleMs
        $note = if ($step.Slot -eq 125) {
            "槽位 125%（实际 zoom=$($step.Zoom)，UI 缩放按 0.1 量化无法取 1.25）"
        } else {
            "槽位 $($step.Slot)%（zoom=$($step.Zoom)）"
        }
        Write-Log 'INFO' "$note → $($step.File)"
        Save-WindowCapture -Hwnd $hwnd -Path (Join-Path $OutDir $step.File)
        Write-Log 'INFO' '等效缩放抽样（zoom 路径），不是系统 DPI 切换'
    }

    $exitCode = 0
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    Add-Content -LiteralPath $script:logPath -Value ('[{0}] FAIL  {1}' -f (Get-Date -Format 'HH:mm:ss.fff'), $_.Exception.Message) -Encoding utf8
    $exitCode = 1
} finally {
    if ($stderrPath -and (Test-Path -LiteralPath $stderrPath)) {
        $info = Get-Item -LiteralPath $stderrPath
        if ($info.Length -gt 0) {
            $tail = @(Get-Content -LiteralPath $stderrPath -Tail 20)
            Write-Log 'INFO' "stderr（最后 $($tail.Count) 行）：$($tail -join ' | ')"
        }
    }
    if ($null -ne $proc -and -not $proc.HasExited) {
        $proc.CloseMainWindow() | Out-Null
        if (-not $proc.WaitForExit(5000)) {
            $proc.Kill()
            $proc.WaitForExit(5000) | Out-Null
        }
    }
    if ($tempRoot -and (Test-Path -LiteralPath $tempRoot)) {
        for ($i = 0; $i -lt 3; $i++) {
            try { Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction Stop; break } catch { Start-Sleep -Milliseconds 400 }
        }
    }
}

exit $exitCode
