# qq-task7 高对比度真机取证（可逆）：临时开启 Windows 高对比度 → 观察 Veil 是否在
# 1s 内切换为 Theme::high_contrast() → 截图 → finally 恢复原始设置并再次截图。
#
# 诚实与安全约束：
#   - 脚本先读取并打印 HKCU\Control Panel\Accessibility\HighContrast 的 Flags 原值；
#   - 若当前已是高对比度，则不做任何改动，直接 NOT RUN（无法证明“切换”）；
#   - 置位走 SystemParametersInfo(SPI_SETHIGHCONTRAST)（系统真实入口，可回滚），
#     并额外广播 WM_SETTINGCHANGE("Accessibility")；
#   - finally 无条件恢复原 Flags（SPI + 注册表双写）并校验；恢复失败会大声报错并 exit 2；
#   - 每一步失败都先恢复再退出，绝不让系统停留在高对比度状态。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/qq_task7_high_contrast.ps1

[CmdletBinding()]
param(
    [string]$Exe = '',
    [string]$Doc = '',
    [string]$OutDir = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'evidence\verify\qq-task7' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$script:logPath = Join-Path $OutDir 'high-contrast.log'

$regPath = 'HKCU:\Control Panel\Accessibility\HighContrast'
$HCF_HIGHCONTRASTON = 0x1

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    Add-Content -LiteralPath $script:logPath -Value $line -Encoding utf8
    Write-Host $line
}

Add-Type -AssemblyName System.Drawing
if (-not ('QQTask7HC.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace QQTask7HC {
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public static class Win32 {
        public delegate bool EnumProc(IntPtr hWnd, IntPtr param);

        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr param);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
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

    public static class Settings {
        const uint SPI_GETHIGHCONTRAST = 0x0042;
        const uint SPI_SETHIGHCONTRAST = 0x0043;
        const uint SPIF_UPDATEINIFILE = 0x0001;
        const uint SPIF_SENDCHANGE = 0x0002;
        const uint SMTO_BLOCK = 0x0001;
        const uint SMTO_ABORTIFHUNG = 0x0002;
        static string scheme;

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct HIGHCONTRAST {
            public uint cbSize;
            public uint dwFlags;
            public IntPtr lpszDefaultScheme;
        }

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool SystemParametersInfo(uint uiAction, uint uiParam, ref HIGHCONTRAST pvParam, uint fWinIni);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, IntPtr wParam, string lParam, uint flags, uint timeout, out IntPtr result);

        public static uint GetHighContrastFlags() {
            HIGHCONTRAST hc = new HIGHCONTRAST();
            hc.cbSize = (uint)Marshal.SizeOf(typeof(HIGHCONTRAST));
            hc.lpszDefaultScheme = IntPtr.Zero;
            if (!SystemParametersInfo(SPI_GETHIGHCONTRAST, hc.cbSize, ref hc, 0)) {
                throw new InvalidOperationException("SPI_GETHIGHCONTRAST failed: " + Marshal.GetLastWin32Error());
            }
            scheme = hc.lpszDefaultScheme != IntPtr.Zero ? Marshal.PtrToStringUni(hc.lpszDefaultScheme) : null;
            return hc.dwFlags;
        }

        public static bool SetHighContrastFlags(uint flags) {
            HIGHCONTRAST hc = new HIGHCONTRAST();
            hc.cbSize = (uint)Marshal.SizeOf(typeof(HIGHCONTRAST));
            hc.dwFlags = flags;
            IntPtr p = IntPtr.Zero;
            if (!string.IsNullOrEmpty(scheme)) { p = Marshal.StringToHGlobalUni(scheme); }
            try {
                hc.lpszDefaultScheme = p;
                return SystemParametersInfo(SPI_SETHIGHCONTRAST, hc.cbSize, ref hc, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE);
            } finally {
                if (p != IntPtr.Zero) { Marshal.FreeHGlobal(p); }
            }
        }

        public static bool BroadcastAccessibilityChange() {
            IntPtr result;
            IntPtr ok = SendMessageTimeout(new IntPtr(0xffff), 0x001A, IntPtr.Zero, "Accessibility",
                SMTO_BLOCK | SMTO_ABORTIFHUNG, 3000, out result);
            return ok != IntPtr.Zero;
        }
    }
}
'@
}

[QQTask7HC.Win32]::SetProcessDPIAware() | Out-Null

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [QQTask7HC.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [QQTask7HC.Win32]::IsWindowVisible($h)) { return $true }
        if ([QQTask7HC.Win32]::IsIconic($h)) { return $true }
        $r = New-Object QQTask7HC.RECT
        [QQTask7HC.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [QQTask7HC.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [QQTask7HC.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
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

function Get-WindowBitmap {
    param([IntPtr]$Hwnd)
    $wr = New-Object QQTask7HC.RECT
    [QQTask7HC.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) { throw "窗口尺寸无效 ${w}x${h}" }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $printed = $false
    try {
        $printed = [QQTask7HC.Win32]::PrintWindow($Hwnd, $hdc, [QQTask7HC.K]::PW_RENDERFULLCONTENT)
    } finally {
        $g.ReleaseHdc($hdc)
        $g.Dispose()
    }
    if (-not $printed) {
        $flags = ([QQTask7HC.K]::SWP_NOMOVE -bor [QQTask7HC.K]::SWP_NOSIZE -bor [QQTask7HC.K]::SWP_NOACTIVATE)
        [QQTask7HC.Win32]::SetWindowPos($Hwnd, [QQTask7HC.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 250
            $g2 = [System.Drawing.Graphics]::FromImage($bmp)
            try { $g2.CopyFromScreen($wr.Left, $wr.Top, 0, 0, (New-Object System.Drawing.Size($w, $h))) } finally { $g2.Dispose() }
        } finally {
            [QQTask7HC.Win32]::SetWindowPos($Hwnd, [QQTask7HC.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        }
    }
    return $bmp
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

function Get-RegistryFlags {
    $item = Get-Item -LiteralPath $regPath -ErrorAction Stop
    return [string]$item.GetValue('Flags')
}

function Set-RegistryFlags {
    param([string]$Value)
    # 用 Set-ItemProperty 走提供程序写入；直接对 Get-Item 返回的键调用 .SetValue
    # 在当前环境会拿到只读句柄（Cannot write to the registry key）。
    Set-ItemProperty -Path $regPath -Name Flags -Value $Value -Type String -ErrorAction Stop
}

# ---------- 预检 ----------
$originalRegistry = Get-RegistryFlags
$originalFlags = [uint32][QQTask7HC.Settings]::GetHighContrastFlags()
Write-Log 'INFO' "原值：Flags(注册表)=$originalRegistry  Flags(SPI)=$originalFlags (0x$('{0:X}' -f $originalFlags))"
Write-Host "当前高对比度 Flags 原值：注册表=$originalRegistry，SPI=$originalFlags"

if (($originalFlags -band $HCF_HIGHCONTRASTON) -ne 0 -or ([int]$originalRegistry -band $HCF_HIGHCONTRASTON) -ne 0) {
    Write-Log 'SKIP' '系统已处于高对比度模式；为避免无意义的状态变更，NOT RUN（无法证明“切换”）'
    Write-Host 'NOT RUN：系统已处于高对比度模式，未做任何修改。'
    exit 0
}

$exitCode = 0
$proc = $null
$tempRoot = ''
$toggled = $false
$restored = $false

function Restore-HighContrast {
    param([uint32]$Flags, [string]$RegistryValue)
    for ($i = 0; $i -lt 5; $i++) {
        try {
            $spiOk = [QQTask7HC.Settings]::SetHighContrastFlags($Flags)
            Set-RegistryFlags -Value $RegistryValue
            [QQTask7HC.Settings]::BroadcastAccessibilityChange() | Out-Null
            Start-Sleep -Milliseconds 400
            $nowSpi = [uint32][QQTask7HC.Settings]::GetHighContrastFlags()
            $nowReg = Get-RegistryFlags
            if ($spiOk -and $nowSpi -eq $Flags -and $nowReg -eq $RegistryValue) {
                return $true
            }
            Write-Log 'WARN' "恢复校验未通过（第 $($i + 1) 次）：SPI=$nowSpi 注册表=$nowReg"
        } catch {
            Write-Log 'WARN' "恢复失败（第 $($i + 1) 次）：$($_.Exception.Message)"
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

try {
    if (-not (Test-Path -LiteralPath $Exe)) { throw "找不到 release 可执行文件：$Exe" }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-qq-task7-hc-{0}-{1}" -f $stamp, $PID)
    $veilHome = Join-Path $tempRoot 'home'
    $appData = Join-Path $tempRoot 'appdata'
    $localAppData = Join-Path $tempRoot 'localappdata'
    $tempDir = Join-Path $tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null

    if (-not $Doc) {
        $readme = Join-Path $repoRoot 'README.zh-CN.md'
        if (-not (Test-Path -LiteralPath $readme)) { throw "缺少 README.zh-CN.md：$readme" }
        $Doc = Join-Path $tempRoot 'qq-task7-hc-doc.md'
        Copy-Item -LiteralPath $readme -Destination $Doc -Force
    }
    if (-not (Test-Path -LiteralPath $Doc)) { throw "找不到 -Doc 文件：$Doc" }

    # 跟随系统（mode=system）：高对比度才能覆盖 light/dark。
    [IO.File]::WriteAllText((Join-Path $veilHome 'config.toml'), "mode = `"system`"`n", [Text.UTF8Encoding]::new($false))
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
    Start-Sleep -Milliseconds 1500

    $baseline = Get-WindowBitmap -Hwnd $hwnd
    $baselineLum = Get-AverageLuminance -Bitmap $baseline
    $baseline.Save((Join-Path $OutDir 'high-contrast-baseline.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $baseline.Dispose()
    Write-Log 'INFO' "切换前基线：平均亮度=$([Math]::Round($baselineLum, 1))（system 模式下应为浅色/深色常态）"

    # ---- 置位高对比度 ----
    $newFlags = $originalFlags -bor $HCF_HIGHCONTRASTON
    $newRegistry = ([int]$originalRegistry -bor $HCF_HIGHCONTRASTON).ToString()
    Set-RegistryFlags -Value $newRegistry
    $spiSet = [QQTask7HC.Settings]::SetHighContrastFlags($newFlags)
    [QQTask7HC.Settings]::BroadcastAccessibilityChange() | Out-Null
    $toggled = $true
    Write-Log 'INFO' "已请求开启高对比度：SPI=$newFlags（接口返回 $spiSet），注册表=$newRegistry；已广播 WM_SETTINGCHANGE(Accessibility)"

    $verifyFlags = [uint32][QQTask7HC.Settings]::GetHighContrastFlags()
    if (($verifyFlags -band $HCF_HIGHCONTRASTON) -eq 0) {
        throw "置位校验失败：SPI 仍报告 Flags=$verifyFlags（高对比度未生效）"
    }

    # 轮询等待 Veil 切换（产品 1s 轮询；上限 4s），记录切色耗时。
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $switchMs = -1
    $hcLum = $baselineLum
    while ($sw.ElapsedMilliseconds -lt 4000) {
        Start-Sleep -Milliseconds 200
        $probe = Get-WindowBitmap -Hwnd $hwnd
        $hcLum = Get-AverageLuminance -Bitmap $probe
        $probe.Dispose()
        if ($hcLum -lt 60) {
            $switchMs = [int]$sw.ElapsedMilliseconds
            break
        }
    }
    $sw.Stop()
    if ($switchMs -ge 0) {
        Write-Log 'PASS' "Veil 已切换为高对比度配色：耗时 ${switchMs}ms，平均亮度 $([Math]::Round($hcLum, 1))"
    } else {
        Write-Log 'FAIL' "4s 内未观察到 Veil 切换（最终平均亮度 $([Math]::Round($hcLum, 1))）"
        throw "Veil 未在 1s 级轮询内切换高对比度配色"
    }

    # 任务要求：置位后等待 ≥2s 再截图。
    $remaining = 2000 - $sw.ElapsedMilliseconds
    if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]$remaining) }

    $hcBmp = Get-WindowBitmap -Hwnd $hwnd
    $hcBmp.Save((Join-Path $OutDir 'high-contrast.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $hcBmp.Dispose()
    Write-Log 'PASS' "已保存 high-contrast.png"

    # ---- 恢复 ----
    $restored = Restore-HighContrast -Flags $originalFlags -RegistryValue $originalRegistry
    if (-not $restored) {
        Write-Host '严重：高对比度恢复失败，请立即手动恢复！' -ForegroundColor Red
        throw '恢复系统高对比度设置失败，请立即手动恢复'
    }
    Write-Log 'PASS' "已恢复原值：Flags=$originalFlags，注册表=$originalRegistry"

    # 恢复后再等主题轮询回落，然后截图证明已还原。
    Start-Sleep -Milliseconds 2000
    $restBmp = Get-WindowBitmap -Hwnd $hwnd
    $restLum = Get-AverageLuminance -Bitmap $restBmp
    $restBmp.Save((Join-Path $OutDir 'high-contrast-restored.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $restBmp.Dispose()
    if ($restLum -lt 60) {
        Write-Log 'FAIL' "恢复后 Veil 仍偏暗（平均亮度 $([Math]::Round($restLum, 1))），主题未回落"
        throw "恢复后 Veil 未回落到原主题（平均亮度 $([Math]::Round($restLum, 1))）"
    }
    Write-Log 'PASS' "已还原：平均亮度 $([Math]::Round($restLum, 1))，high-contrast-restored.png 已保存"

    $exitCode = 0
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    Add-Content -LiteralPath $script:logPath -Value ('[{0}] FAIL  {1}' -f (Get-Date -Format 'HH:mm:ss.fff'), $_.Exception.Message) -Encoding utf8
    $exitCode = 1
} finally {
    # 无条件兜底恢复：只要动过就必须还原并校验。
    if ($toggled -and -not $restored) {
        $ok = Restore-HighContrast -Flags $originalFlags -RegistryValue $originalRegistry
        if ($ok) {
            $restored = $true
            Write-Log 'PASS' 'finally 兜底恢复成功'
        } else {
            Write-Host '严重：无法恢复高对比度设置（注册表原值见上方日志），请手动恢复！' -ForegroundColor Red
            Write-Log 'FAIL' "finally 兜底恢复失败：请手动把 $regPath 的 Flags 设为 $originalRegistry"
            $exitCode = 2
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
