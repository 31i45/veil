# Veil DPI assisted 取证（Task 5）
#
# 用户只做一件事：把 Windows「显示 → 缩放」切到目标档位（切换后需重新登录或重启 veil.exe）。
# 脚本负责：读取当前系统/窗口 DPI、核对请求档位、启动 release veil.exe、等待首帧稳定、
# 用 .NET System.Drawing 抓窗口截图、把结果追加到 dpi.jsonl。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/assisted/dpi.ps1                    # 只报告当前 DPI
#   pwsh -NoProfile -File tools/verify/assisted/dpi.ps1 -Dpi 150 -Capture  # 当前档为 150% 时抓图
#   pwsh -NoProfile -File tools/verify/assisted/dpi.ps1 -Dpi 125 -Capture  # 当前不是 125% → NOT RUN
#
# 纪律：请求档位与实测 DPI 不一致时一律写 NOT RUN 并 exit 2，绝不拿别的档位截图冒充。
#       未切换的档位不产生 png，只写一行带 status=NOT_RUN 的 JSON。
#
# 退出码：0 = 已抓图/已报告且档位一致；2 = 档位不一致（NOT RUN）；1 = 执行失败。

[CmdletBinding()]
param(
    [ValidateSet('100', '125', '150', '200')]
    [string]$Dpi = '',
    [switch]$Capture,
    [string]$Exe = '',
    [string]$Doc = '',
    [string]$OutDir = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$dateStamp = '2026-11-27'
if (-not $OutDir) { $OutDir = Join-Path $repoRoot "docs\qa\evidence\assisted-$dateStamp" }
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$jsonlPath = Join-Path $OutDir 'dpi.jsonl'

Add-Type -AssemblyName System.Drawing
if (-not ('VeilAssistedDpi.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace VeilAssistedDpi {
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public static class Native {
        public delegate bool EnumProc(IntPtr hWnd, IntPtr param);

        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr param);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT value, int size);
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);
        [DllImport("gdi32.dll")] public static extern int GetDeviceCaps(IntPtr hdc, int index);
        [DllImport("user32.dll")] public static extern int GetDpiForSystem();
        [DllImport("user32.dll")] public static extern int GetDpiForWindow(IntPtr hWnd);
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

[VeilAssistedDpi.Native]::SetProcessDPIAware() | Out-Null

function Get-SystemDpi {
    $dc = [VeilAssistedDpi.Native]::GetDC([IntPtr]::Zero)
    try {
        $logx = [VeilAssistedDpi.Native]::GetDeviceCaps($dc, 88)  # LOGPIXELSX
        $logy = [VeilAssistedDpi.Native]::GetDeviceCaps($dc, 90)  # LOGPIXELSY
    } finally {
        [void][VeilAssistedDpi.Native]::ReleaseDC([IntPtr]::Zero, $dc)
    }
    $system = 0
    try { $system = [VeilAssistedDpi.Native]::GetDpiForSystem() } catch { }
    [pscustomobject]@{ LogPixelsX = $logx; LogPixelsY = $logy; SystemDpi = $system; Percent = [math]::Round($logx / 96.0 * 100) }
}

function Get-RegistryLogPixels {
    try {
        $v = (Get-ItemProperty -Path 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction Stop).LogPixels
        return [int]$v
    } catch {
        return $null
    }
}

function Write-Record {
    param([hashtable]$Fields)
    $line = $Fields | ConvertTo-Json -Compress -Depth 4
    Add-Content -LiteralPath $jsonlPath -Value $line -Encoding utf8
    Write-Host "记录：$jsonlPath"
}

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = [System.Collections.Generic.List[object]]::new()
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilAssistedDpi.Native]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [VeilAssistedDpi.Native]::IsWindowVisible($h)) { return $true }
        if ([VeilAssistedDpi.Native]::IsIconic($h)) { return $true }
        $r = New-Object VeilAssistedDpi.RECT
        [VeilAssistedDpi.Native]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [VeilAssistedDpi.Native]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [VeilAssistedDpi.Native]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
    return @($script:enumWindows | Sort-Object { $_.Hwnd.ToInt64() })
}

function Wait-MainWindow {
    param($Process, [int]$TimeoutMs)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        if ($Process.HasExited) { throw "进程在窗口出现前退出：PID=$($Process.Id) ExitCode=$($Process.ExitCode)" }
        $windows = @(Get-ProcessWindows -ProcessId $Process.Id)
        if ($windows.Count -gt 0) { return $windows[0] }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    throw "等待主窗口超时（${TimeoutMs}ms）"
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

function Save-WindowCapture {
    param([IntPtr]$Hwnd, [string]$Path)
    $wr = New-Object VeilAssistedDpi.RECT
    [VeilAssistedDpi.Native]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) { throw "窗口尺寸无效 ${w}x${h}" }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $printed = $false
    for ($attempt = 0; $attempt -lt 3 -and -not $printed; $attempt++) {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        try {
            $printed = [VeilAssistedDpi.Native]::PrintWindow($Hwnd, $hdc, [VeilAssistedDpi.K]::PW_RENDERFULLCONTENT)
        } finally {
            $g.ReleaseHdc($hdc)
            $g.Dispose()
        }
        if ($printed -and (Test-BlankBitmap -Bitmap $bmp)) {
            $printed = $false
            Start-Sleep -Milliseconds 350
        }
    }
    $method = 'PrintWindow'
    $blank = if ($printed) { Test-BlankBitmap -Bitmap $bmp } else { $true }
    if (-not $printed -or $blank) {
        $visible = New-Object VeilAssistedDpi.RECT
        $dwmOk = [VeilAssistedDpi.Native]::DwmGetWindowAttribute($Hwnd, 9, [ref]$visible, 16) -eq 0
        if (-not $dwmOk -or ($visible.Right - $visible.Left) -le 0 -or ($visible.Bottom - $visible.Top) -le 0) {
            $visible = $wr
        }
        $vw = $visible.Right - $visible.Left
        $vh = $visible.Bottom - $visible.Top
        $flags = ([VeilAssistedDpi.K]::SWP_NOMOVE -bor [VeilAssistedDpi.K]::SWP_NOSIZE -bor [VeilAssistedDpi.K]::SWP_NOACTIVATE)
        $screen = New-Object System.Drawing.Bitmap($vw, $vh)
        [VeilAssistedDpi.Native]::SetWindowPos($Hwnd, [VeilAssistedDpi.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 300
            $g2 = [System.Drawing.Graphics]::FromImage($screen)
            try {
                $g2.CopyFromScreen($visible.Left, $visible.Top, 0, 0, (New-Object System.Drawing.Size($vw, $vh)))
            } finally { $g2.Dispose() }
        } finally {
            [VeilAssistedDpi.Native]::SetWindowPos($Hwnd, [VeilAssistedDpi.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        }
        $bmp.Dispose()
        $bmp = $screen
        $method = 'CopyFromScreen'
        $blank = Test-BlankBitmap -Bitmap $bmp
    }
    if ($blank) {
        $blankPath = [IO.Path]::ChangeExtension($Path, '.blank.png')
        $bmp.Save($blankPath, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        throw "截图全黑/纯色，已保留诊断图：$blankPath"
    }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $width = $bmp.Width
    $height = $bmp.Height
    $bmp.Dispose()
    [pscustomobject]@{
        Path = $Path; Bytes = (Get-Item -LiteralPath $Path).Length
        Width = $width; Height = $height; Method = $method
    }
}

$stamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
$sys = Get-SystemDpi
$regLogPixels = Get-RegistryLogPixels
Write-Host "当前系统 DPI：LogPixelsX=$($sys.LogPixelsX)（$($sys.Percent)%），GetDpiForSystem=$($sys.SystemDpi)，注册表 LogPixels=$(if ($null -eq $regLogPixels) { '未设置' } else { $regLogPixels })"

if (-not $Dpi) {
    Write-Host '未指定 -Dpi：只报告当前档位。抓图请加 -Dpi <100|125|150|200> -Capture。'
    Write-Record @{
        time = $stamp; status = 'REPORT_ONLY'; requested_dpi = $null
        observed_percent = $sys.Percent; logpixels_x = $sys.LogPixelsX
        system_dpi = $sys.SystemDpi; registry_logpixels = $regLogPixels
        screenshot = $null; machine = $env:COMPUTERNAME
    }
    exit 0
}

$requested = [int]$Dpi
if ($sys.Percent -ne $requested) {
    $msg = "当前系统缩放为 $($sys.Percent)%，与请求的 $requested% 不一致。"
    Write-Host $msg -ForegroundColor Yellow
    Write-Host '请执行（唯一需要你做的物理动作）：'
    Write-Host '  1) Windows 设置 → 系统 → 屏幕 → 缩放，选择目标百分比；'
    Write-Host '  2) 重新登录 Windows（或至少完全退出并重启 veil.exe）让所有进程换用新 DPI；'
    Write-Host "  3) 重新运行：pwsh -NoProfile -File tools/verify/assisted/dpi.ps1 -Dpi $requested -Capture"
    Write-Record @{
        time = $stamp; status = 'NOT_RUN'; requested_dpi = $requested
        observed_percent = $sys.Percent; logpixels_x = $sys.LogPixelsX
        system_dpi = $sys.SystemDpi; registry_logpixels = $regLogPixels
        screenshot = $null; reason = $msg; machine = $env:COMPUTERNAME
    }
    exit 2
}

if (-not $Capture) {
    Write-Host "档位一致（$requested%）。加 -Capture 才会启动 veil.exe 并抓图。"
    Write-Record @{
        time = $stamp; status = 'READY'; requested_dpi = $requested
        observed_percent = $sys.Percent; logpixels_x = $sys.LogPixelsX
        system_dpi = $sys.SystemDpi; registry_logpixels = $regLogPixels
        screenshot = $null; machine = $env:COMPUTERNAME
    }
    exit 0
}

if (-not (Test-Path -LiteralPath $Exe)) {
    Write-Host "找不到 release 可执行文件：$Exe" -ForegroundColor Red
    Write-Host '请先构建：cargo build --release -p veil'
    Write-Record @{
        time = $stamp; status = 'FAIL'; requested_dpi = $requested
        observed_percent = $sys.Percent; screenshot = $null
        reason = "缺少 $Exe，请先 cargo build --release -p veil"; machine = $env:COMPUTERNAME
    }
    exit 1
}
if (-not $Doc) {
    $matrix = Join-Path $repoRoot 'docs\qa\syntax-matrix.md'
    $Doc = if (Test-Path -LiteralPath $matrix) { $matrix } else { Join-Path $repoRoot 'README.zh-CN.md' }
}
if (-not (Test-Path -LiteralPath $Doc)) { throw "找不到测试语料：$Doc（docs/qa/syntax-matrix.md 与 README.zh-CN.md 均不存在）" }

$pngPath = Join-Path $OutDir "dpi-$requested.png"
$proc = $null
$tempRoot = ''
try {
    $stampIso = Get-Date -Format 'yyyyMMdd-HHmmss'
    $tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-dpi-$stampIso-$PID")
    $veilHome = Join-Path $tempRoot 'home'
    $appData = Join-Path $tempRoot 'appdata'
    $localAppData = Join-Path $tempRoot 'localappdata'
    $tempDir = Join-Path $tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null

    $proc = Start-Process -FilePath $Exe -ArgumentList @($Doc) -PassThru `
        -RedirectStandardOutput (Join-Path $tempRoot 'stdout.log') `
        -RedirectStandardError (Join-Path $tempRoot 'stderr.log') `
        -Environment @{
            VEIL_HOME = $veilHome; APPDATA = $appData; LOCALAPPDATA = $localAppData
            TEMP = $tempDir; TMP = $tempDir
        }
    $window = Wait-MainWindow -Process $proc -TimeoutMs 25000
    $hwnd = [IntPtr]$window.Hwnd

    # 首帧稳定：窗口出现后再等 1.5 s，让首帧排版/光栅与字体加载完成。
    Start-Sleep -Milliseconds 1500
    [void][VeilAssistedDpi.Native]::ShowWindow($hwnd, [VeilAssistedDpi.K]::SW_RESTORE)
    for ($i = 0; $i -lt 5; $i++) {
        [void][VeilAssistedDpi.Native]::SetForegroundWindow($hwnd)
        Start-Sleep -Milliseconds 150
        if ([VeilAssistedDpi.Native]::GetForegroundWindow() -eq $hwnd) { break }
    }

    $windowDpi = 0
    try { $windowDpi = [VeilAssistedDpi.Native]::GetDpiForWindow($hwnd) } catch { }
    $windowPercent = if ($windowDpi -gt 0) { [math]::Round($windowDpi / 96.0 * 100) } else { $sys.Percent }
    if ($windowPercent -ne $requested) {
        $msg = "窗口 DPI=$windowDpi（$windowPercent%）与请求 $requested% 不一致：应用可能开在另一块缩放不同的显示器上，拒绝保存冒充证据。"
        Write-Host $msg -ForegroundColor Yellow
        Write-Record @{
            time = $stamp; status = 'NOT_RUN'; requested_dpi = $requested
            observed_percent = $sys.Percent; logpixels_x = $sys.LogPixelsX
            system_dpi = $sys.SystemDpi; window_dpi = $windowDpi; window_percent = $windowPercent
            registry_logpixels = $regLogPixels; screenshot = $null; reason = $msg
            machine = $env:COMPUTERNAME
        }
        exit 2
    }

    $shot = Save-WindowCapture -Hwnd $hwnd -Path $pngPath
    Write-Host "已抓图：$($shot.Path)（$($shot.Width)x$($shot.Height)，$($shot.Bytes) B，方法 $($shot.Method)）"
    Write-Record @{
        time = $stamp; status = 'PASS'; requested_dpi = $requested
        observed_percent = $sys.Percent; logpixels_x = $sys.LogPixelsX
        system_dpi = $sys.SystemDpi; window_dpi = $windowDpi; window_percent = $windowPercent
        registry_logpixels = $regLogPixels; screenshot = $pngPath
        png_bytes = $shot.Bytes; window = "$($shot.Width)x$($shot.Height)"
        capture_method = $shot.Method; corpus = $Doc; machine = $env:COMPUTERNAME
    }
    exit 0
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    Write-Record @{
        time = $stamp; status = 'FAIL'; requested_dpi = $requested
        observed_percent = $sys.Percent; screenshot = $null
        reason = $_.Exception.Message; machine = $env:COMPUTERNAME
    }
    exit 1
} finally {
    if ($null -ne $proc -and -not $proc.HasExited) {
        [void]$proc.CloseMainWindow()
        if (-not $proc.WaitForExit(5000)) { try { $proc.Kill() } catch { } }
    }
    if ($tempRoot -and (Test-Path -LiteralPath $tempRoot)) {
        for ($i = 0; $i -lt 3; $i++) {
            try { Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction Stop; break } catch { Start-Sleep -Milliseconds 400 }
        }
    }
}
