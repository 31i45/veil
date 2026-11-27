# 高对比度三表面取证（可逆，证据脚本，非产品代码）：
# 临时开启 Windows 高对比度 → 启动 Veil（隔离 VEIL_HOME）→ 截图编辑器 →
# 写出 hc-ready.txt 等待人工/代理截取命令面板与删除确认 → 收到 hc-done.txt 或超时后
# 无条件恢复原高对比度设置并校验。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/qq_task7_high_contrast_surfaces.ps1 [-WaitSeconds 240]

[CmdletBinding()]
param(
    [string]$Exe = '',
    [string]$OutDir = '',
    [int]$WaitSeconds = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'docs\qa\evidence\verify\qq-task7-hc-surfaces' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$script:logPath = Join-Path $OutDir 'high-contrast.log'
$readyPath = Join-Path $OutDir 'hc-ready.txt'
$donePath = Join-Path $OutDir 'hc-done.txt'
Remove-Item -LiteralPath $readyPath, $donePath -Force -ErrorAction SilentlyContinue

$regPath = 'HKCU:\Control Panel\Accessibility\HighContrast'
$HCF_HIGHCONTRASTON = 0x1

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    Add-Content -LiteralPath $script:logPath -Value $line -Encoding utf8
    Write-Host $line
}

Add-Type -AssemblyName System.Drawing
if (-not ('QQTask7HCS.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

namespace QQTask7HCS {
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
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);

        public static ulong Fnv64(string text) {
            ulong h = 14695981039346656037UL;
            foreach (byte b in Encoding.UTF8.GetBytes(text)) {
                h ^= b;
                h *= 1099511628211UL;
            }
            return h;
        }
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

[QQTask7HCS.Win32]::SetProcessDPIAware() | Out-Null

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [QQTask7HCS.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [QQTask7HCS.Win32]::IsWindowVisible($h)) { return $true }
        if ([QQTask7HCS.Win32]::IsIconic($h)) { return $true }
        $r = New-Object QQTask7HCS.RECT
        [QQTask7HCS.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [QQTask7HCS.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [QQTask7HCS.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
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

function Get-WindowBitmap {
    param([IntPtr]$Hwnd)
    $wr = New-Object QQTask7HCS.RECT
    [QQTask7HCS.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) { throw "窗口尺寸无效 ${w}x${h}" }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $printed = $false
    try {
        $printed = [QQTask7HCS.Win32]::PrintWindow($Hwnd, $hdc, [QQTask7HCS.K]::PW_RENDERFULLCONTENT)
    } finally {
        $g.ReleaseHdc($hdc)
        $g.Dispose()
    }
    if (-not $printed) {
        $flags = ([QQTask7HCS.K]::SWP_NOMOVE -bor [QQTask7HCS.K]::SWP_NOSIZE -bor [QQTask7HCS.K]::SWP_NOACTIVATE)
        [QQTask7HCS.Win32]::SetWindowPos($Hwnd, [QQTask7HCS.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 250
            $g2 = [System.Drawing.Graphics]::FromImage($bmp)
            try { $g2.CopyFromScreen($wr.Left, $wr.Top, 0, 0, (New-Object System.Drawing.Size($w, $h))) } finally { $g2.Dispose() }
        } finally {
            [QQTask7HCS.Win32]::SetWindowPos($Hwnd, [QQTask7HCS.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
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
    Set-ItemProperty -Path $regPath -Name Flags -Value $Value -Type String -ErrorAction Stop
}

function Restore-HighContrast {
    param([uint32]$Flags, [string]$RegistryValue)
    for ($i = 0; $i -lt 5; $i++) {
        try {
            $spiOk = [QQTask7HCS.Settings]::SetHighContrastFlags($Flags)
            Set-RegistryFlags -Value $RegistryValue
            [QQTask7HCS.Settings]::BroadcastAccessibilityChange() | Out-Null
            Start-Sleep -Milliseconds 400
            $nowSpi = [uint32][QQTask7HCS.Settings]::GetHighContrastFlags()
            $nowReg = Get-RegistryFlags
            if ($spiOk -and $nowSpi -eq $Flags -and $nowReg -eq $RegistryValue) { return $true }
            Write-Log 'WARN' "恢复校验未通过（第 $($i + 1) 次）：SPI=$nowSpi 注册表=$nowReg"
        } catch {
            Write-Log 'WARN' "恢复失败（第 $($i + 1) 次）：$($_.Exception.Message)"
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

$originalRegistry = Get-RegistryFlags
$originalFlags = [uint32][QQTask7HCS.Settings]::GetHighContrastFlags()
Write-Log 'INFO' "原值：Flags(注册表)=$originalRegistry Flags(SPI)=$originalFlags"
if (($originalFlags -band $HCF_HIGHCONTRASTON) -ne 0 -or ([int]$originalRegistry -band $HCF_HIGHCONTRASTON) -ne 0) {
    Write-Log 'SKIP' '系统已处于高对比度模式；NOT RUN（不改变现有状态）'
    exit 0
}

$exitCode = 0
$proc = $null
$tempRoot = ''
$toggled = $false
$restored = $false

try {
    if (-not (Test-Path -LiteralPath $Exe)) { throw "找不到 release 可执行文件：$Exe" }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-hc-surfaces-{0}-{1}" -f $stamp, $PID)
    $veilHome = Join-Path $tempRoot 'home'
    $appData = Join-Path $tempRoot 'appdata'
    $localAppData = Join-Path $tempRoot 'localappdata'
    $tempDir = Join-Path $tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null

    $doc = Join-Path $tempRoot 'hc-surfaces.md'
    $docText = @"
# 高对比度取证

普通段落：The quick brown fox 跳过了懒狗。

## 列表与代码

- 列表项一
- 列表项二

``````rust
fn main() {
    println!("hc");
}
``````

> 引用块：命令面板与删除确认应保持可读。
"@
    [IO.File]::WriteAllText($doc, $docText, [Text.UTF8Encoding]::new($false))

    # 预置两个版本快照，供“删除快照”确认弹窗使用（引擎缺 meta 时会按正文重建）。
    $normalized = $doc.ToLowerInvariant().Replace('/', '\')
    $key = [QQTask7HCS.Win32]::Fnv64($normalized)
    $snapDir = Join-Path $veilHome ('snapshots\{0:x16}' -f $key)
    New-Item -ItemType Directory -Path $snapDir -Force | Out-Null
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    [IO.File]::WriteAllText((Join-Path $snapDir ('{0:x16}.md' -f ($now - 60000))), "# 版本一`n`n旧内容。`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $snapDir ('{0:x16}.md' -f ($now - 30000))), "# 版本二`n`n较新的内容。`n", [Text.UTF8Encoding]::new($false))
    Write-Log 'INFO' "预置快照：$snapDir"

    [IO.File]::WriteAllText((Join-Path $veilHome 'config.toml'), "mode = `"system`"`n", [Text.UTF8Encoding]::new($false))

    $proc = Start-Process -FilePath $Exe -ArgumentList @($doc) -PassThru `
        -RedirectStandardOutput (Join-Path $tempRoot 'stdout.log') -RedirectStandardError (Join-Path $tempRoot 'stderr.log') `
        -Environment @{
            VEIL_HOME = $veilHome
            APPDATA = $appData
            LOCALAPPDATA = $localAppData
            TEMP = $tempDir
            TMP = $tempDir
        }
    Write-Log 'INFO' "已启动 Veil：PID=$($proc.Id) 文档=$doc"

    $window = Wait-MainWindow -Process $proc -TimeoutMs 25000
    $hwnd = [IntPtr]$window.Hwnd
    Write-Log 'INFO' "主窗口：HWND=$hwnd 标题=$($window.Title)"
    Start-Sleep -Milliseconds 1500

    $newFlags = $originalFlags -bor $HCF_HIGHCONTRASTON
    $newRegistry = ([int]$originalRegistry -bor $HCF_HIGHCONTRASTON).ToString()
    Set-RegistryFlags -Value $newRegistry
    $spiSet = [QQTask7HCS.Settings]::SetHighContrastFlags($newFlags)
    [QQTask7HCS.Settings]::BroadcastAccessibilityChange() | Out-Null
    $toggled = $true
    Write-Log 'INFO' "已开启高对比度：SPI=$newFlags（返回 $spiSet）注册表=$newRegistry"

    $verifyFlags = [uint32][QQTask7HCS.Settings]::GetHighContrastFlags()
    if (($verifyFlags -band $HCF_HIGHCONTRASTON) -eq 0) { throw "置位校验失败：Flags=$verifyFlags" }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $hcLum = 255.0
    while ($sw.ElapsedMilliseconds -lt 5000) {
        Start-Sleep -Milliseconds 200
        $probe = Get-WindowBitmap -Hwnd $hwnd
        $hcLum = Get-AverageLuminance -Bitmap $probe
        $probe.Dispose()
        if ($hcLum -lt 60) { break }
    }
    if ($hcLum -ge 60) { throw "5s 内未观察到 Veil 切换高对比度（平均亮度 $hcLum）" }
    Start-Sleep -Milliseconds 1200

    [QQTask7HCS.Win32]::ShowWindow($hwnd, 3) | Out-Null
    [QQTask7HCS.Win32]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
    [QQTask7HCS.Win32]::SetForegroundWindow($hwnd) | Out-Null
    [QQTask7HCS.Win32]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 700

    $editor = Get-WindowBitmap -Hwnd $hwnd
    $editor.Save((Join-Path $OutDir 'hc-editor.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $editor.Dispose()
    Write-Log 'PASS' "已保存 hc-editor.png（平均亮度 $([Math]::Round($hcLum, 1))）"

    # 预开版本历史面板，减少人工/代理步骤；面板内容也留一张高对比度证据。
    $ws = New-Object -ComObject WScript.Shell
    $ws.AppActivate($proc.Id) | Out-Null
    Start-Sleep -Milliseconds 400
    $ws.SendKeys("^+h") | Out-Null
    Start-Sleep -Milliseconds 1200
    $history = Get-WindowBitmap -Hwnd $hwnd
    $history.Save((Join-Path $OutDir 'hc-history.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $history.Dispose()
    Write-Log 'PASS' '已打开版本历史并保存 hc-history.png'

    Set-Content -LiteralPath $readyPath -Value ("pid={0}`nhwnd={1}" -f $proc.Id, $hwnd) -Encoding utf8
    Write-Log 'INFO' "等待命令面板/删除确认截图完成：最多 ${WaitSeconds}s（done 文件 $donePath）"

    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $donePath)) {
        Start-Sleep -Milliseconds 500
        if ($proc.HasExited) { throw "取证期间 Veil 已退出" }
    }
    if (Test-Path -LiteralPath $donePath) {
        Write-Log 'PASS' '收到 hc-done.txt'
    } else {
        Write-Log 'WARN' "等待超时（${WaitSeconds}s），按计划恢复"
    }

    $restored = Restore-HighContrast -Flags $originalFlags -RegistryValue $originalRegistry
    if (-not $restored) { throw '恢复系统高对比度设置失败，请立即手动恢复' }
    Write-Log 'PASS' "已恢复原值：Flags=$originalFlags 注册表=$originalRegistry"
    Start-Sleep -Milliseconds 1200
    $final = Get-WindowBitmap -Hwnd $hwnd
    $final.Save((Join-Path $OutDir 'hc-restored.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $finalLum = Get-AverageLuminance -Bitmap $final
    $final.Dispose()
    Write-Log 'INFO' "恢复后窗口平均亮度=$([Math]::Round($finalLum, 1))"
    $exitCode = 0
} catch {
    Write-Log 'FAIL' $_.Exception.Message
    $exitCode = 1
} finally {
    if ($toggled -and -not $restored) {
        $ok = Restore-HighContrast -Flags $originalFlags -RegistryValue $originalRegistry
        if ($ok) { $restored = $true; Write-Log 'PASS' 'finally 兜底恢复成功' }
        else { Write-Log 'FAIL' "finally 兜底恢复失败：请手动把 $regPath 的 Flags 设为 $originalRegistry"; $exitCode = 2 }
    }
    if ($null -ne $proc -and -not $proc.HasExited) {
        $proc.CloseMainWindow() | Out-Null
        if (-not $proc.WaitForExit(5000)) { $proc.Kill(); $proc.WaitForExit(5000) | Out-Null }
    }
    if ($tempRoot -and (Test-Path -LiteralPath $tempRoot)) {
        for ($i = 0; $i -lt 3; $i++) {
            try { Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction Stop; break } catch { Start-Sleep -Milliseconds 400 }
        }
    }
}

exit $exitCode
