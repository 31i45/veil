# Veil 真机走查驱动脚本（证据工具，非 CI 门禁）
#
# 用法示例：
#   pwsh -File tools/walkthrough.ps1 -Doc "$env:TEMP\veil-walkthrough\02-混合元素.md" -Widths 240,320,480,800 -ScrollPages 3 -Probe
#   pwsh -File tools/walkthrough.ps1 -Doc "$env:TEMP\veil-walkthrough\03-10万字.txt" -Measure open,mem -KeepAlive
#   pwsh -File tools/walkthrough.ps1 -Doc "$env:TEMP\veil-walkthrough\03-10万字.txt" -Perf
#
# 说明：
#   - 按 PID 定位窗口，不依赖前台焦点；滚动用 PostMessage，不抢用户焦点。
#   - 截图用 PrintWindow（PW_RENDERFULLCONTENT），窗口被遮挡也能取到内容。
#   - 结束时默认结束目标进程；-KeepAlive 可保留窗口供人工观察。
#   - 任何失败都会写入 walkthrough.log，并区分「脚本失败」与「产品行为」。
#
# Win10 冒烟（启动 / 打开 / 保存 / 打印 / 导出 PDF，交互桌面执行）：
#   pwsh -File tools/walkthrough.ps1 -Doc "$env:TEMP\veil-smoke\smoke.md" -Steps "wait:1500;click:25,15;wait:400;key:^s;wait:800;shot:save;click:25,15;wait:400;key:{DOWN}{DOWN}{DOWN}{DOWN}{DOWN}{DOWN}{ENTER};wait:5000;shot:done;click:25,15;wait:400;key:{DOWN}{DOWN}{DOWN}{DOWN}{DOWN}{DOWN}{DOWN}{ENTER};wait:2000;shot-screen:print"
#   期望：保存步骤后 dirty 标记消失；导出步骤后同目录出现 `<stem>.pdf`（以 %PDF 开头）；
#   打印步骤开系统对话框（shot-screen 取证，人工关闭）。菜单项顺序见 chrome::MENUS

[CmdletBinding()]
param(
    [string]$Exe = "",
    [string]$Doc = "",
    [string]$Widths = "240,320,480,800",
    [int]$ScrollPages = 0,
    [string]$Steps = "",
    [int]$StepsWidth = 0,
    [string]$ShotDir = "",
    [switch]$Probe,
    [switch]$Measure,
    [switch]$Perf,
    [int]$PerfChars = 200,
    [switch]$KeepAlive,
    [int]$SettleMs = 700,
    [int]$WindowWaitMs = 15000,
    [string[]]$AuditFiles = @()
)

$ErrorActionPreference = 'Stop'

# ---------- 路径与日志 ----------
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not (Test-Path -LiteralPath $Exe)) {
    throw "脚本失败：找不到 exe：$Exe（请先 cargo build --release -p veil）"
}
if ($Doc -and -not (Test-Path -LiteralPath $Doc)) {
    throw "脚本失败：找不到文档：$Doc"
}

$widthList = @($Widths -split '[,;\s]+' | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [int]$_ })
if ($widthList.Count -eq 0) {
    throw "脚本失败：-Widths 没有可解析的宽度（收到：$Widths）"
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if (-not $ShotDir) {
    $ShotDir = Join-Path $env:TEMP "veil-walkthrough\evidence\run-$stamp"
}
New-Item -ItemType Directory -Path $ShotDir -Force | Out-Null
$logPath = Join-Path $ShotDir 'walkthrough.log'
$stdoutPath = Join-Path $ShotDir 'stdout.log'
$stderrPath = Join-Path $ShotDir 'stderr.log'

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = "[{0}] {1,-5} {2}" -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    Add-Content -LiteralPath $logPath -Value $line
    Write-Host $line
}

# ---------- Win32 ----------
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
if (-not ('VeilWalkthrough.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace VeilWalkthrough {
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static class Win32 {
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern bool GetCursorPos(ref POINT p);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);

        [StructLayout(LayoutKind.Sequential)]
        public struct POINT { public int X; public int Y; }

        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr param);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);

        public delegate bool EnumProc(IntPtr hWnd, IntPtr param);

        public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
        public const uint MOUSEEVENTF_LEFTUP = 0x0004;
        public const uint MOUSEEVENTF_RIGHTDOWN = 0x0008;
        public const uint MOUSEEVENTF_RIGHTUP = 0x0010;
        public const byte VK_MENU = 0x12;
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    }

    public static class Const {
        public const uint WM_KEYDOWN = 0x0100;
        public const uint WM_KEYUP = 0x0101;
        public const uint WM_MOUSEWHEEL = 0x020A;
        public const uint WM_CLOSE = 0x0010;
        public const int VK_NEXT = 0x22;
        public const int VK_PRIOR = 0x21;
        public const int SW_RESTORE = 9;
        public const uint SWP_NOMOVE = 0x0002;
        public const uint SWP_NOZORDER = 0x0004;
        public const uint SWP_NOSIZE = 0x0001;
        public const uint SWP_NOACTIVATE = 0x0010;
        public const uint PW_RENDERFULLCONTENT = 2;
        public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
        public static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    }
}
'@
}

# 高 DPI 屏（如 150%）下 DPI 不感知的 pwsh 会拿到虚拟坐标：
# GetWindowRect/CopyFromScreen 混用会让截图只覆盖窗口左上角的一部分（实测丢掉状态栏）。
# 进程内声明 DPI 感知后，窗口矩形、客户端矩形、点击坐标、屏幕捕获统一为物理像素。
[VeilWalkthrough.Win32]::SetProcessDPIAware() | Out-Null

# 枚举同一 PID 的全部可见顶层文档窗口。Process.MainWindowHandle 只返回其中一个，
# 不能用于单进程多窗口验收；句柄排序仅保证步骤中的窗口编号在枚举间稳定。
function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilWalkthrough.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [VeilWalkthrough.Win32]::IsWindowVisible($h) -or
            [VeilWalkthrough.Win32]::IsIconic($h)) { return $true }
        $wr = New-Object VeilWalkthrough.RECT
        [VeilWalkthrough.Win32]::GetWindowRect($h, [ref]$wr) | Out-Null
        if (($wr.Right - $wr.Left) -lt 200 -or ($wr.Bottom - $wr.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 1024
        [VeilWalkthrough.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{
            Hwnd = $h
            Title = $title.ToString()
            Left = $wr.Left
            Top = $wr.Top
            Right = $wr.Right
            Bottom = $wr.Bottom
        })
        return $true
    }
    [VeilWalkthrough.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
    return @($script:enumWindows | Sort-Object { $_.Hwnd.ToInt64() })
}

function Wait-ProcessWindowCount {
    param([int]$ProcessId, [int]$MinimumCount, [int]$TimeoutMs)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        if ($null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { return $null }
        $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
        if ($windows.Count -ge $MinimumCount) { return $windows }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    return $null
}

function Get-MainWindow {
    param([int]$ProcessId, [int]$TimeoutMs)
    $windows = Wait-ProcessWindowCount -ProcessId $ProcessId -MinimumCount 1 -TimeoutMs $TimeoutMs
    if ($null -eq $windows -or @($windows).Count -eq 0) { return $null }
    return @($windows)[0].Hwnd
}

function Wait-WindowStable {
    param([IntPtr]$Hwnd, [int]$TimeoutMs)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        $wr = New-Object VeilWalkthrough.RECT
        [VeilWalkthrough.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
        $h = $wr.Bottom - $wr.Top
        $visible = [VeilWalkthrough.Win32]::IsWindowVisible($Hwnd)
        $iconic = [VeilWalkthrough.Win32]::IsIconic($Hwnd)
        if ($visible -and -not $iconic -and $h -ge 300) { return $true }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Set-ClientWidth {
    param([IntPtr]$Hwnd, [int]$ClientWidth)
    $wr = New-Object VeilWalkthrough.RECT
    $cr = New-Object VeilWalkthrough.RECT
    [VeilWalkthrough.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    [VeilWalkthrough.Win32]::GetClientRect($Hwnd, [ref]$cr) | Out-Null
    $outerW = $wr.Right - $wr.Left
    $outerH = $wr.Bottom - $wr.Top
    # 启动早期窗口可能是临时小尺寸；高度不可信时用常规窗口高度兜底
    if ($outerH -lt 300) { $outerH = 900 }
    $clientW = $cr.Right - $cr.Left
    $nonClient = [Math]::Max($outerW - $clientW, 0)
    $target = [Math]::Max($ClientWidth + $nonClient, 200)
    [VeilWalkthrough.Win32]::SetWindowPos(
        $Hwnd, [IntPtr]::Zero, 0, 0, $target, $outerH,
        ([VeilWalkthrough.Const]::SWP_NOMOVE -bor [VeilWalkthrough.Const]::SWP_NOZORDER)
    ) | Out-Null
    Start-Sleep -Milliseconds $SettleMs
}

function Test-BlankBitmap {
    param([System.Drawing.Bitmap]$Bmp)
    $colors = @{}
    $stepX = [Math]::Max(1, [int]($Bmp.Width / 16))
    $stepY = [Math]::Max(1, [int]($Bmp.Height / 10))
    for ($y = 2; $y -lt $Bmp.Height; $y += $stepY) {
        for ($x = 2; $x -lt $Bmp.Width; $x += $stepX) {
            $colors[$Bmp.GetPixel($x, $y).ToArgb()] = $true
        }
    }
    return ($colors.Count -le 2)
}

function Save-WindowScreenshot {
    param([IntPtr]$Hwnd, [string]$Path)
    $wr = New-Object VeilWalkthrough.RECT
    [VeilWalkthrough.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) {
        Write-Log 'WARN' "截图失败：窗口尺寸无效 ${w}x${h}"
        return $false
    }
    $dpi = [VeilWalkthrough.Win32]::GetDpiForWindow($Hwnd)
    Write-Log 'INFO' "真实窗口 DPI=$dpi（比例=$([Math]::Round($dpi / 96.0, 3))，物理像素=${w}x${h}，句柄=$Hwnd）"
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [VeilWalkthrough.Win32]::PrintWindow($Hwnd, $hdc, [VeilWalkthrough.Const]::PW_RENDERFULLCONTENT)
    $g.ReleaseHdc($hdc)
    $g.Dispose()

    $blank = $true
    if ($ok) { $blank = Test-BlankBitmap -Bmp $bmp }

    if (-not $ok -or $blank) {
        # 自绘窗口 PrintWindow 常返回全黑：临时置顶后屏幕捕获，随后恢复 z-order。
        # 不调用 SetForegroundWindow，避免抢走用户焦点。
        $flags = ([VeilWalkthrough.Const]::SWP_NOMOVE -bor [VeilWalkthrough.Const]::SWP_NOSIZE -bor [VeilWalkthrough.Const]::SWP_NOACTIVATE)
        [VeilWalkthrough.Win32]::SetWindowPos($Hwnd, [VeilWalkthrough.Const]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        Start-Sleep -Milliseconds 250
        $g2 = [System.Drawing.Graphics]::FromImage($bmp)
        $g2.CopyFromScreen($wr.Left, $wr.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $g2.Dispose()
        [VeilWalkthrough.Win32]::SetWindowPos($Hwnd, [VeilWalkthrough.Const]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        Write-Log 'WARN' "PrintWindow 无效（ok=$ok, blank=$blank），已降级为置顶屏幕捕获（未抢焦点）：$Path"
        $ok = $false
    }

    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $ok
}

function Send-PageKey {
    param([IntPtr]$Hwnd, [int]$VirtualKey, [int]$Times)
    for ($i = 0; $i -lt $Times; $i++) {
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYDOWN, [IntPtr]$VirtualKey, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 40
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYUP, [IntPtr]$VirtualKey, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 240
    }
}

function Send-Wheel {
    param([IntPtr]$Hwnd, [int]$Notches)
    $wParam = [IntPtr]::new((0 - 120 * $Notches) * 65536)
    for ($i = 0; $i -lt $Notches; $i++) {
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_MOUSEWHEEL, $wParam, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 120
    }
}

# 前台化（交互型步骤专用）：返回原前台窗口，结束后由调用方还原。
# Windows 前台锁会拒绝非前台进程的 SetForegroundWindow，失败时用 Alt 轻敲解锁后重试。
function Enter-Foreground {
    param([IntPtr]$Hwnd)
    $prev = [VeilWalkthrough.Win32]::GetForegroundWindow()
    if ($prev -eq $Hwnd) { return $prev }
    [VeilWalkthrough.Win32]::ShowWindow($Hwnd, [VeilWalkthrough.Const]::SW_RESTORE) | Out-Null
    # 前台锁下先做一次 topmost 往返并允许激活；真实点击/ SendKeys 才不会落到遮挡窗口。
    $activateFlags = ([VeilWalkthrough.Const]::SWP_NOMOVE -bor [VeilWalkthrough.Const]::SWP_NOSIZE)
    [VeilWalkthrough.Win32]::SetWindowPos($Hwnd, [VeilWalkthrough.Const]::HWND_TOPMOST, 0, 0, 0, 0, $activateFlags) | Out-Null
    [VeilWalkthrough.Win32]::SetWindowPos($Hwnd, [VeilWalkthrough.Const]::HWND_NOTOPMOST, 0, 0, 0, 0, $activateFlags) | Out-Null
    [VeilWalkthrough.Win32]::SetForegroundWindow($Hwnd) | Out-Null
    Start-Sleep -Milliseconds 350
    if ([VeilWalkthrough.Win32]::GetForegroundWindow() -ne $Hwnd) {
        [VeilWalkthrough.Win32]::keybd_event([VeilWalkthrough.Win32]::VK_MENU, 0, 0, [UIntPtr]::Zero)
        [VeilWalkthrough.Win32]::keybd_event([VeilWalkthrough.Win32]::VK_MENU, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 120
        [VeilWalkthrough.Win32]::SetForegroundWindow($Hwnd) | Out-Null
        Start-Sleep -Milliseconds 350
    }
    return $prev
}

function Send-RealClick {
    param([IntPtr]$Hwnd, [int]$X, [int]$Y, [switch]$Right)
    $pt = New-Object VeilWalkthrough.Win32+POINT
    $pt.X = $X
    $pt.Y = $Y
    [VeilWalkthrough.Win32]::ClientToScreen($Hwnd, [ref]$pt) | Out-Null
    [VeilWalkthrough.Win32]::SetCursorPos($pt.X, $pt.Y) | Out-Null
    Start-Sleep -Milliseconds 120
    $down = if ($Right) { [VeilWalkthrough.Win32]::MOUSEEVENTF_RIGHTDOWN } else { [VeilWalkthrough.Win32]::MOUSEEVENTF_LEFTDOWN }
    $up = if ($Right) { [VeilWalkthrough.Win32]::MOUSEEVENTF_RIGHTUP } else { [VeilWalkthrough.Win32]::MOUSEEVENTF_LEFTUP }
    [VeilWalkthrough.Win32]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [VeilWalkthrough.Win32]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 300
}

# 显式 postclick/postrclick/postkey 步骤：锁屏或无法前台化时，仍由真实目标窗口处理输入。
# 不改变全局光标/前台，不伪造应用画面；日志与最终证据必须保留所用的窗口消息方式。
function Send-WindowMessageClick {
    param([IntPtr]$Hwnd, [int]$X, [int]$Y, [switch]$Right)
    $lParam = [IntPtr](($Y -band 0xFFFF) * 65536 + ($X -band 0xFFFF))
    $move = 0x0200
    $down = if ($Right) { 0x0204 } else { 0x0201 }
    $up = if ($Right) { 0x0205 } else { 0x0202 }
    $button = if ($Right) { [IntPtr]2 } else { [IntPtr]1 }
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, $move, [IntPtr]::Zero, $lParam) | Out-Null
    Start-Sleep -Milliseconds 80
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, $down, $button, $lParam) | Out-Null
    Start-Sleep -Milliseconds 80
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, $up, [IntPtr]::Zero, $lParam) | Out-Null
    Start-Sleep -Milliseconds 300
}

function Send-WindowMessageDrag {
    param([IntPtr]$Hwnd, [int]$X1, [int]$Y1, [int]$X2, [int]$Y2)
    $start = [IntPtr](($Y1 -band 0xFFFF) * 65536 + ($X1 -band 0xFFFF))
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, 0x0200, [IntPtr]::Zero, $start) | Out-Null
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, 0x0201, [IntPtr]1, $start) | Out-Null
    for ($i = 1; $i -le 16; $i++) {
        $x = $X1 + [int](($X2 - $X1) * $i / 16)
        $y = $Y1 + [int](($Y2 - $Y1) * $i / 16)
        $point = [IntPtr](($y -band 0xFFFF) * 65536 + ($x -band 0xFFFF))
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, 0x0200, [IntPtr]1, $point) | Out-Null
        Start-Sleep -Milliseconds 25
    }
    $end = [IntPtr](($Y2 -band 0xFFFF) * 65536 + ($X2 -band 0xFFFF))
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, 0x0202, [IntPtr]::Zero, $end) | Out-Null
    Start-Sleep -Milliseconds 400
}

function Send-WindowMessageKey {
    param([IntPtr]$Hwnd, [string]$Chord)
    $keys = @{ ESC = 0x1B; ENTER = 0x0D; HOME = 0x24; END = 0x23; PGUP = 0x21; PGDN = 0x22; DOWN = 0x28 }
    $key = $Chord.ToUpperInvariant()
    if (-not $keys.ContainsKey($key)) { throw "postkey 不支持的键：$Chord" }
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYDOWN, [IntPtr]$keys[$key], [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds 60
    [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYUP, [IntPtr]$keys[$key], [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds 300
}

# 屏幕捕获（始终 CopyFromScreen）：只取目标进程可见窗口的并集，
# 既覆盖 MessageBox 等独立窗口，也不把其它应用截进证据。
function Save-ProcessScreenshot {
    param([IntPtr]$Hwnd, [string]$Path)
    $targetPid = [uint32]0
    [VeilWalkthrough.Win32]::GetWindowThreadProcessId($Hwnd, [ref]$targetPid) | Out-Null
    $script:capPid = $targetPid
    $script:capRect = @([int]::MaxValue, [int]::MaxValue, [int]::MinValue, [int]::MinValue)
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilWalkthrough.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:capPid) { return $true }
        if (-not [VeilWalkthrough.Win32]::IsWindowVisible($h)) { return $true }
        $r = New-Object VeilWalkthrough.RECT
        [VeilWalkthrough.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 80 -or ($r.Bottom - $r.Top) -lt 60) { return $true }
        if ($r.Left -lt $script:capRect[0]) { $script:capRect[0] = $r.Left }
        if ($r.Top -lt $script:capRect[1]) { $script:capRect[1] = $r.Top }
        if ($r.Right -gt $script:capRect[2]) { $script:capRect[2] = $r.Right }
        if ($r.Bottom -gt $script:capRect[3]) { $script:capRect[3] = $r.Bottom }
        return $true
    }
    [VeilWalkthrough.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
    $x0 = $script:capRect[0]
    $y0 = $script:capRect[1]
    $x1 = $script:capRect[2]
    $y1 = $script:capRect[3]
    if ($x1 -le $x0 -or $y1 -le $y0) {
        Write-Log 'WARN' '屏幕捕获失败：未找到目标进程可见窗口'
        return
    }
    $w = $x1 - $x0
    $h = $y1 - $y0
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x0, $y0, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose()
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Log 'INFO' "进程窗口并集截图 ${w}x${h}（含独立对话框）"
}


function Wait-ProcessWindowExactCount {
    param([int]$ProcessId, [int]$ExpectedCount, [int]$TimeoutMs)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        if ($null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { return $null }
        $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
        if ($windows.Count -eq $ExpectedCount) {
            # 多等一个重绘周期再复核，避免关窗事件尚未路由时把旧数量当成结论。
            Start-Sleep -Milliseconds 350
            $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
            if ($windows.Count -eq $ExpectedCount) { return $windows }
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    return $null
}

function Invoke-Secondary {
    param([int]$PrimaryId, [string[]]$Arguments, [int]$TimeoutMs)
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmssfff'
    $stdout = Join-Path $ShotDir "secondary-$stamp.stdout.log"
    $stderr = Join-Path $ShotDir "secondary-$stamp.stderr.log"
    $startArgs = @{
        FilePath = $Exe
        ArgumentList = $Arguments
        PassThru = $true
        RedirectStandardOutput = $stdout
        RedirectStandardError = $stderr
    }
    $secondary = Start-Process @startArgs
    $secondaryId = $secondary.Id
    $argText = $Arguments -join ' '
    if (-not $secondary.WaitForExit($TimeoutMs)) {
        Stop-Process -Id $secondaryId -Force -ErrorAction SilentlyContinue
        throw "次实例 PID=$secondaryId 未在 ${TimeoutMs}ms 内退出（参数：$argText）"
    }
    Write-Log 'INFO' "次实例已退出：PID=$secondaryId ExitCode=$($secondary.ExitCode) 参数=$argText"
    if ($secondary.ExitCode -ne 0) {
        throw "次实例 PID=$secondaryId 退出码 $($secondary.ExitCode)（参数：$argText）"
    }
    $live = @(Get-Process -Name veil -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe })
    $liveText = @($live | ForEach-Object { "PID=$($_.Id)" }) -join '，'
    Write-Log 'INFO' "次实例退出后的同路径 Veil 进程数=$($live.Count)：$liveText"
    if ($live.Count -ne 1 -or $live[0].Id -ne $PrimaryId) {
        throw "单实例进程断言失败：期望仅主进程 PID=$PrimaryId，实际=$liveText"
    }
}

# 步骤串：click:x,y / rclick:x,y / key:发送键串（SendKeys 语法，如 {TAB} ^p；{SPACE} 走 VK_SPACE） / type:文本 / wait:毫秒
#   postclick/postrclick:x,y / postdrag:x1,y1,x2,y2 / postkey:ESC|ENTER|HOME|END|PGUP|PGDN|DOWN：显式向目标窗口投递输入（锁屏走查用）。
#   shot:名字 / shot-screen:名字 / wait-windows:最小数量 / expect-windows:精确数量 / window:从0开始的编号
#   window-title:唯一标题子串 / expect-title:当前窗口完整标题 / close / wait-exit / shot-windows:名字
#   tile-windows：同 PID 文档窗口横向平铺，避免 PrintWindow 降级截图时相互遮挡。
#   launch-new / launch-doc:路径 / launch-new-doc:路径：启动次实例并断言其退出且主进程唯一。
#   shot 走 PrintWindow（窗口内容）；shot-screen 走屏幕捕获（用于系统对话框等独立窗口）。
#   window/window-title 切换后续交互目标并前台化；shot-windows 对同 PID 的每个文档窗口分别截图。
function Invoke-Steps {
    param([IntPtr]$Hwnd, [int]$ProcessId, [string]$Spec, [string]$Dir, [string]$Stamp, [ref]$Index)
    $targetHwnd = $Hwnd
    foreach ($raw in @($Spec -split ';')) {
        $step = $raw.Trim()
        if (-not $step) { continue }
        $kind, $arg = $step.Split(':', 2)
        switch ($kind) {
            'click' {
                $xy = $arg.Split(',')
                Send-RealClick -Hwnd $targetHwnd -X ([int]$xy[0]) -Y ([int]$xy[1])
                Write-Log 'INFO' "点击 $arg"
            }
            'rclick' {
                $xy = $arg.Split(',')
                Send-RealClick -Hwnd $targetHwnd -X ([int]$xy[0]) -Y ([int]$xy[1]) -Right
                Write-Log 'INFO' "右键 $arg"
            }
            'postclick' {
                $xy = $arg.Split(',')
                Send-WindowMessageClick -Hwnd $targetHwnd -X ([int]$xy[0]) -Y ([int]$xy[1])
                Write-Log 'INFO' "窗口消息左键 $arg"
            }
            'postrclick' {
                $xy = $arg.Split(',')
                Send-WindowMessageClick -Hwnd $targetHwnd -X ([int]$xy[0]) -Y ([int]$xy[1]) -Right
                Write-Log 'INFO' "窗口消息右键 $arg"
            }
            'postdrag' {
                $xy = $arg.Split(',')
                Send-WindowMessageDrag -Hwnd $targetHwnd -X1 ([int]$xy[0]) -Y1 ([int]$xy[1]) -X2 ([int]$xy[2]) -Y2 ([int]$xy[3])
                Write-Log 'INFO' "窗口消息拖动 $arg"
            }
            'postkey' {
                Send-WindowMessageKey -Hwnd $targetHwnd -Chord $arg
                Write-Log 'INFO' "窗口消息按键 $arg"
            }
            'hover' {
                # 仅移动鼠标（不按键）：客户端坐标转屏幕后 SetCursorPos，
                # 再发一次 MOUSEEVENTF_MOVE(0x0001) 触发 WM_MOUSEMOVE；等 400ms 让悬停态重绘。
                $xy = $arg.Split(',')
                $pt = New-Object VeilWalkthrough.Win32+POINT
                $pt.X = [int]$xy[0]
                $pt.Y = [int]$xy[1]
                [VeilWalkthrough.Win32]::ClientToScreen($targetHwnd, [ref]$pt) | Out-Null
                [VeilWalkthrough.Win32]::SetCursorPos($pt.X, $pt.Y) | Out-Null
                Start-Sleep -Milliseconds 60
                # 相对移动 2px 再回到目标点：确保产生真实 WM_MOUSEMOVE（SetCursorPos 单独调用未必触发重绘）。
                [VeilWalkthrough.Win32]::mouse_event(0x0001, 1, 0, 0, [UIntPtr]::Zero)
                [VeilWalkthrough.Win32]::mouse_event(0x0001, 1, 0, 0, [UIntPtr]::Zero)
                Start-Sleep -Milliseconds 120
                [VeilWalkthrough.Win32]::SetCursorPos($pt.X, $pt.Y) | Out-Null
                $chk = New-Object VeilWalkthrough.Win32+POINT
                [VeilWalkthrough.Win32]::GetCursorPos([ref]$chk) | Out-Null
                Start-Sleep -Milliseconds 400
                Write-Log 'INFO' "悬停 $arg 目标屏幕=($($pt.X),$($pt.Y)) 实际光标=($($chk.X),$($chk.Y))"
            }
            'pastefile' {
                # 以资源管理器复制的形式把文件放进剪贴板（文件列表），再 ^v 让应用走图片/附件粘贴。
                $paths = @($arg -split ';' | Where-Object { $_ })
                $list = New-Object System.Collections.Specialized.StringCollection
                foreach ($f in $paths) { $list.Add((Resolve-Path -LiteralPath $f).Path) }
                [System.Windows.Forms.Clipboard]::SetFileDropList($list)
                Start-Sleep -Milliseconds 120
                [System.Windows.Forms.SendKeys]::SendWait('^v')
                Start-Sleep -Milliseconds 600
                Write-Log 'INFO' "粘贴文件 $arg"
            }
            'launch-new' {
                Invoke-Secondary -PrimaryId $ProcessId -Arguments @('--new-window') -TimeoutMs $WindowWaitMs
            }
            'launch-doc' {
                Invoke-Secondary -PrimaryId $ProcessId -Arguments @($arg) -TimeoutMs $WindowWaitMs
            }
            'launch-new-doc' {
                Invoke-Secondary -PrimaryId $ProcessId -Arguments @('--new-window', $arg) -TimeoutMs $WindowWaitMs
            }
            'key' {
                $foreground = [VeilWalkthrough.Win32]::GetForegroundWindow()
                if ($foreground -ne $targetHwnd) {
                    Write-Log 'WARN' "按键前目标窗口不在前台：目标=$targetHwnd 前台=$foreground，重新前台化"
                    Enter-Foreground -Hwnd $targetHwnd | Out-Null
                }
                if ($arg -ceq '{SPACE}') {
                    # SendKeys 没有可移植的 {SPACE} 键名；直接发送命名键，避免中文输入法组合态吞掉普通字符。
                    [VeilWalkthrough.Win32]::keybd_event(0x20, 0, 0, [UIntPtr]::Zero)
                    Start-Sleep -Milliseconds 40
                    [VeilWalkthrough.Win32]::keybd_event(0x20, 0, 2, [UIntPtr]::Zero)
                } else {
                    [System.Windows.Forms.SendKeys]::SendWait($arg)
                }
                Start-Sleep -Milliseconds 400
                $foreground = [VeilWalkthrough.Win32]::GetForegroundWindow()
                Write-Log 'INFO' "按键 $arg（前台=$foreground，目标=$targetHwnd）"
            }
            'type' {
                # 走剪贴板粘贴：SendKeys 逐字符会被输入法截获成分解候选，ESC 还会吞掉文本。
                $old = $null
                try { $old = [System.Windows.Forms.Clipboard]::GetText() } catch { $old = $null }
                [System.Windows.Forms.Clipboard]::SetText($arg)
                Start-Sleep -Milliseconds 120
                [System.Windows.Forms.SendKeys]::SendWait('^v')
                Start-Sleep -Milliseconds 400
                if ($old) {
                    try { [System.Windows.Forms.Clipboard]::SetText($old) } catch { }
                }
                Write-Log 'INFO' "输入文本（剪贴板粘贴）"
            }
            'wait-windows' {
                $count = [int]$arg
                if ($count -lt 1) { throw "wait-windows 数量必须大于 0：$arg" }
                $windows = Wait-ProcessWindowCount -ProcessId $ProcessId -MinimumCount $count -TimeoutMs $WindowWaitMs
                if ($null -eq $windows) { throw "等待 ${count} 个文档窗口超时（${WindowWaitMs}ms）" }
                $description = @($windows | ForEach-Object { "句柄=$($_.Hwnd) 标题=$($_.Title)" }) -join '；'
                Write-Log 'INFO' "文档窗口数=$(@($windows).Count)：$description"
            }
            'tile-windows' {
                $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
                if ($windows.Count -eq 0) { throw '平铺失败：当前 PID 无文档窗口' }
                $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
                $margin = 8
                $gap = 8
                $width = [int](($area.Width - 2 * $margin - ($windows.Count - 1) * $gap) / $windows.Count)
                $height = $area.Height - 2 * $margin
                for ($n = 0; $n -lt $windows.Count; $n++) {
                    $x = $area.X + $margin + $n * ($width + $gap)
                    $y = $area.Y + $margin
                    [VeilWalkthrough.Win32]::SetWindowPos(
                        $windows[$n].Hwnd, [IntPtr]::Zero, $x, $y, $width, $height,
                        ([VeilWalkthrough.Const]::SWP_NOZORDER -bor [VeilWalkthrough.Const]::SWP_NOACTIVATE)
                    ) | Out-Null
                    Write-Log 'INFO' "平铺窗口[$n]：句柄=$($windows[$n].Hwnd) 标题=$($windows[$n].Title) 矩形=${x},${y},${width},${height}"
                }
                Start-Sleep -Milliseconds 500
            }
            'expect-windows' {
                $count = [int]$arg
                if ($count -lt 1) { throw "expect-windows 数量必须大于 0：$arg" }
                $windows = Wait-ProcessWindowExactCount -ProcessId $ProcessId -ExpectedCount $count -TimeoutMs $WindowWaitMs
                if ($null -eq $windows) {
                    $actual = @(Get-ProcessWindows -ProcessId $ProcessId)
                    $actualText = @($actual | ForEach-Object { "句柄=$($_.Hwnd) 标题=$($_.Title)" }) -join '；'
                    $Index.Value++
                    $failureShot = Join-Path $Dir ("{0:00}-step-expect-windows-timeout-{1}.png" -f $Index.Value, $Stamp)
                    Save-WindowScreenshot -Hwnd $targetHwnd -Path $failureShot | Out-Null
                    $foreground = [VeilWalkthrough.Win32]::GetForegroundWindow()
                    throw "等待精确 ${count} 个文档窗口超时（${WindowWaitMs}ms）；实际=$($actual.Count)：$actualText；前台=$foreground 目标=$targetHwnd；截图=$failureShot"
                }
                $description = @($windows | ForEach-Object { "句柄=$($_.Hwnd) 标题=$($_.Title)" }) -join '；'
                Write-Log 'PASS' "精确文档窗口数=$count：$description"
            }
            'window' {
                $n = [int]$arg
                $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
                if ($n -lt 0 -or $n -ge $windows.Count) {
                    throw "窗口编号 $n 越界（当前 $($windows.Count) 个文档窗口）"
                }
                $targetHwnd = $windows[$n].Hwnd
                Enter-Foreground -Hwnd $targetHwnd | Out-Null
                Write-Log 'INFO' "切换窗口[$n]：句柄=$targetHwnd 标题=$($windows[$n].Title)"
            }
            'window-title' {
                $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
                $titleMatches = @($windows | Where-Object { $_.Title.Contains($arg) })
                if ($titleMatches.Count -ne 1) {
                    $description = @($windows | ForEach-Object { $_.Title }) -join '；'
                    throw "标题子串 '$arg' 应唯一命中，实际 $($titleMatches.Count) 个；全部标题：$description"
                }
                $targetHwnd = $titleMatches[0].Hwnd
                Enter-Foreground -Hwnd $targetHwnd | Out-Null
                Write-Log 'INFO' "按标题切换窗口：句柄=$targetHwnd 标题=$($titleMatches[0].Title)"
            }
            'window-exact' {
                $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
                $titleMatches = @($windows | Where-Object { $_.Title -ceq $arg })
                if ($titleMatches.Count -ne 1) {
                    $description = @($windows | ForEach-Object { $_.Title }) -join '；'
                    throw "完整标题 '$arg' 应唯一命中，实际 $($titleMatches.Count) 个；全部标题：$description"
                }
                $targetHwnd = $titleMatches[0].Hwnd
                Enter-Foreground -Hwnd $targetHwnd | Out-Null
                Write-Log 'INFO' "按完整标题切换窗口：句柄=$targetHwnd 标题=$($titleMatches[0].Title)"
            }
            'expect-title' {
                $current = @(Get-ProcessWindows -ProcessId $ProcessId | Where-Object { $_.Hwnd -eq $targetHwnd })
                if ($current.Count -ne 1 -or $current[0].Title -cne $arg) {
                    $actual = if ($current.Count -eq 1) { $current[0].Title } else { '<窗口不存在>' }
                    $Index.Value++
                    $failureShot = Join-Path $Dir ("{0:00}-step-expect-title-failure-{1}.png" -f $Index.Value, $Stamp)
                    Save-WindowScreenshot -Hwnd $targetHwnd -Path $failureShot | Out-Null
                    throw "窗口标题断言失败：期望='$arg'，实际='$actual'；截图=$failureShot"
                }
                Write-Log 'PASS' "窗口标题=$arg"
            }
            'close' {
                [VeilWalkthrough.Win32]::PostMessage($targetHwnd, [VeilWalkthrough.Const]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
                Write-Log 'INFO' "发送 WM_CLOSE：句柄=$targetHwnd"
            }
            'wait-exit' {
                $primary = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
                if ($primary -and -not $primary.WaitForExit($WindowWaitMs)) {
                    throw "最后窗口关闭后主进程 PID=$ProcessId 未在 ${WindowWaitMs}ms 内退出"
                }
                Write-Log 'PASS' "最后窗口关闭后主进程 PID=$ProcessId 已退出"
            }
            'wait' {
                Start-Sleep -Milliseconds ([int]$arg)
            }
            'hash-file' {
                Write-FileDigest -Path $arg -Phase $step
            }
            'shot' {
                $Index.Value++
                $path = Join-Path $Dir ("{0:00}-step-{1}-{2}.png" -f $Index.Value, $arg, $Stamp)
                Save-WindowScreenshot -Hwnd $targetHwnd -Path $path | Out-Null
                Write-Log 'INFO' "步骤截图：$path"
            }
            'shot-windows' {
                $windows = @(Get-ProcessWindows -ProcessId $ProcessId)
                if ($windows.Count -eq 0) { throw '逐窗口截图失败：当前 PID 无文档窗口' }
                for ($n = 0; $n -lt $windows.Count; $n++) {
                    $Index.Value++
                    $path = Join-Path $Dir ("{0:00}-step-{1}-window-{2}-{3}.png" -f $Index.Value, $arg, $n, $Stamp)
                    Save-WindowScreenshot -Hwnd $windows[$n].Hwnd -Path $path | Out-Null
                    Write-Log 'INFO' "窗口[$n]截图：句柄=$($windows[$n].Hwnd) 标题=$($windows[$n].Title) 文件=$path"
                }
            }
            'shot-screen' {
                $Index.Value++
                $path = Join-Path $Dir ("{0:00}-step-{1}-{2}.png" -f $Index.Value, $arg, $Stamp)
                Save-ProcessScreenshot -Hwnd $targetHwnd -Path $path
                Write-Log 'INFO' "屏幕截图（含独立窗口）：$path"
            }
            default { Write-Log 'WARN' "未知步骤：$step" }
        }
    }
}



function Write-FileDigest {
    param([string]$Path, [string]$Phase)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "文件字节审计失败：找不到 $Path"
    }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    $base64 = [Convert]::ToBase64String($bytes)
    Write-Log 'INFO' "文件字节[$Phase]：路径=$Path 字节数=$($bytes.Length) SHA256=$hash Base64=$base64"
}

function Get-ShotHash {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    (Get-FileHash -LiteralPath $Path -Algorithm MD5).Hash
}

# ---------- 性能探针场景（-Perf） ----------

# 最近秩百分位（对样本量小的情况也稳定）：p50/p95/p99 均取排序后的 ceil(p*n)-1。
function Get-Percentile {
    param([double[]]$Values, [double]$Percent)
    if ($null -eq $Values -or $Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $rank = [Math]::Ceiling(($Percent / 100) * $sorted.Count) - 1
    $idx = [Math]::Min([Math]::Max([int]$rank, 0), $sorted.Count - 1)
    return $sorted[$idx]
}

# 固定键入序列 → 收集 stderr 中的 `[perf] {json}` → 打印分位数 / RSS / 重扫比例。
# 探针行由 veil-ui::perf 输出，字段均为整数（key_us/frame_us/rss_mb/dirty_blocks/
# total_blocks/rescan_lines/undo_depth）；rescan 比例以源文档行数为分母。
function Invoke-PerfScenario {
    param([IntPtr]$Hwnd, [string]$StderrPath, [string]$DocPath, [int]$Chars)
    # 逐字符 PostMessage 到目标窗口：不依赖前台焦点，也不抢用户焦点
    # （SendKeys 需要前台焦点，前台锁会让键入落到别处而采集不到 key_us）。
    # 只发 A–Z 小写字母，避免修饰键与输入法组合。
    $text = 'a' * [Math]::Max($Chars, 1)
    foreach ($ch in $text.ToCharArray()) {
        $vk = [int][char]([char]::ToUpperInvariant($ch))
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYDOWN, [IntPtr]$vk, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 12
        [VeilWalkthrough.Win32]::PostMessage($Hwnd, [VeilWalkthrough.Const]::WM_KEYUP, [IntPtr]$vk, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 4
    }
    Start-Sleep -Milliseconds 1200

    if (-not (Test-Path -LiteralPath $StderrPath)) {
        throw "脚本失败：找不到 stderr 日志：$StderrPath"
    }
    $perfLines = @(Get-Content -LiteralPath $StderrPath -ErrorAction SilentlyContinue |
        Where-Object { $_ -match '^\[perf\] ' })
    if ($perfLines.Count -eq 0) {
        throw "脚本失败：未采集到 [perf] 行（VEIL_PERF_PROBE=1 未生效或探针未启用；见 $StderrPath）"
    }

    $records = @()
    foreach ($line in $perfLines) {
        $json = $line.Substring(7)
        try {
            $records += ($json | ConvertFrom-Json)
        } catch {
            throw "脚本失败：探针行不是合法 JSON：$line"
        }
    }

    $keyUs = @($records | Where-Object { $_ -and ($_.PSObject.Properties.Name -contains 'key_us') } |
        ForEach-Object { [double]$_.key_us })
    $frameUs = @($records | Where-Object { $_ -and ($_.PSObject.Properties.Name -contains 'frame_us') } |
        ForEach-Object { [double]$_.frame_us })
    $rss = @($records | Where-Object { $_ -and ($_.PSObject.Properties.Name -contains 'rss_mb') } |
        ForEach-Object { [double]$_.rss_mb })
    $rescan = @($records | Where-Object { $_ -and ($_.PSObject.Properties.Name -contains 'rescan_lines') } |
        ForEach-Object { [double]$_.rescan_lines })

    if ($keyUs.Count -eq 0) {
        throw "脚本失败：探针输出中没有 key_us 样本（共 $($perfLines.Count) 条 [perf] 行）"
    }

    $avgRss = if ($rss.Count -gt 0) { [Math]::Round(($rss | Measure-Object -Average).Average, 1) } else { $null }
    $avgRescan = if ($rescan.Count -gt 0) { ($rescan | Measure-Object -Average).Average } else { $null }
    $ratioText = 'n/a'
    if ($null -ne $avgRescan -and $DocPath -and (Test-Path -LiteralPath $DocPath)) {
        $totalLines = @(Get-Content -LiteralPath $DocPath -ErrorAction SilentlyContinue).Count
        if ($totalLines -gt 0) { $ratioText = ('{0:P2}' -f ($avgRescan / $totalLines)) }
    }

    Write-Log 'METRIC' "perf samples key=$($keyUs.Count) frame=$($frameUs.Count) edit=$($rescan.Count) lines=$($perfLines.Count)"
    Write-Log 'METRIC' ("perf key_us  p50={0} p95={1} p99={2}" -f (Get-Percentile $keyUs 50), (Get-Percentile $keyUs 95), (Get-Percentile $keyUs 99))
    if ($frameUs.Count -gt 0) {
        Write-Log 'METRIC' ("perf frame_us p50={0} p95={1} p99={2}" -f (Get-Percentile $frameUs 50), (Get-Percentile $frameUs 95), (Get-Percentile $frameUs 99))
    } else {
        Write-Log 'WARN' '未采集到 frame_us 样本（窗口可能未重绘）'
    }
    Write-Log 'METRIC' "perf rss_mb avg=$avgRss"
    Write-Log 'METRIC' "perf rescan_lines avg=$avgRescan ratio=$ratioText"
}

# ---------- 主流程 ----------
$oldProbe = $env:VEIL_BREAK_PROBE
if ($Probe) { $env:VEIL_BREAK_PROBE = '1' }
$oldPerf = $env:VEIL_PERF_PROBE
if ($Perf) { $env:VEIL_PERF_PROBE = '1' }

$proc = $null
$procId = 0
try {
    Write-Log 'INFO' "exe=$Exe doc=$Doc probe=$($Probe.IsPresent) shots=$ShotDir"
    foreach ($auditFile in $AuditFiles) {
        Write-FileDigest -Path $auditFile -Phase '启动前'
    }
    $startArgs = @{
        FilePath = $Exe
        PassThru = $true
        RedirectStandardOutput = $stdoutPath
        RedirectStandardError = $stderrPath
    }
    if ($Doc) { $startArgs.ArgumentList = @($Doc) }

    $openStart = Get-Date
    $proc = Start-Process @startArgs
    $procId = $proc.Id
    Write-Log 'INFO' "已启动 PID=$procId"

    $hwnd = Get-MainWindow -ProcessId $procId -TimeoutMs $WindowWaitMs
    $openMs = [int]((Get-Date) - $openStart).TotalMilliseconds
    if ($null -eq $hwnd -or $hwnd -eq 0) {
        Write-Log 'FAIL' "脚本失败：${WindowWaitMs}ms 内未取得主窗口（可能启动崩溃，见 stderr.log）"
        throw '未取得主窗口'
    }
    Write-Log 'INFO' "主窗口就绪：句柄=$hwnd，启动到窗口 $openMs ms"
    [VeilWalkthrough.Win32]::ShowWindow($hwnd, [VeilWalkthrough.Const]::SW_RESTORE) | Out-Null
    if (-not (Wait-WindowStable -Hwnd $hwnd -TimeoutMs 3000)) {
        Write-Log 'WARN' '窗口在 3s 内未达到稳定可见尺寸（可能仍在首帧布局，截图可能不可信）'
    } else {
        Start-Sleep -Milliseconds $SettleMs
    }

    if ($Perf) {
        Write-Log 'INFO' "性能场景：VEIL_PERF_PROBE=1，固定键入 $PerfChars 字符"
        Invoke-PerfScenario -Hwnd $hwnd -StderrPath $stderrPath -DocPath $Doc -Chars $PerfChars
        return
    }

    $shotIndex = 0
    foreach ($width in $widthList) {
        Set-ClientWidth -Hwnd $hwnd -ClientWidth $width
        $shotIndex++
        $shot = Join-Path $ShotDir ("{0:00}-w{1}-p0-{2}.png" -f $shotIndex, $width, $stamp)
        $ok = Save-WindowScreenshot -Hwnd $hwnd -Path $shot
        Write-Log 'INFO' "宽度 ${width}px 截图：$shot（PrintWindow=$ok）"

        if ($Steps -and ($StepsWidth -eq 0 -or $StepsWidth -eq $width)) {
            $prevFg = Enter-Foreground -Hwnd $hwnd
            $saved = New-Object VeilWalkthrough.Win32+POINT
            [VeilWalkthrough.Win32]::GetCursorPos([ref]$saved) | Out-Null
            try {
                Invoke-Steps -Hwnd $hwnd -ProcessId $procId -Spec $Steps -Dir $ShotDir -Stamp $stamp -Index ([ref]$shotIndex)
            } finally {
                [VeilWalkthrough.Win32]::SetCursorPos($saved.X, $saved.Y) | Out-Null
                if ($prevFg -and $prevFg -ne [IntPtr]::Zero) {
                    [VeilWalkthrough.Win32]::SetForegroundWindow($prevFg) | Out-Null
                }
                Write-Log 'INFO' '交互步骤完成，前台与鼠标位置已还原'
            }
        }

        if ($ScrollPages -gt 0) {
            $prevShot = $shot
            $stuck = 0
            for ($page = 1; $page -le $ScrollPages; $page++) {
                Send-PageKey -Hwnd $hwnd -VirtualKey ([VeilWalkthrough.Const]::VK_NEXT) -Times 1
                $shotIndex++
                $shot = Join-Path $ShotDir ("{0:00}-w{1}-p{2}-{3}.png" -f $shotIndex, $width, $page, $stamp)
                Save-WindowScreenshot -Hwnd $hwnd -Path $shot | Out-Null
                Write-Log 'INFO' "宽度 ${width}px 第 $page 页截图：$shot"
                if ((Get-ShotHash $prevShot) -eq (Get-ShotHash $shot)) {
                    # 无焦点窗口可能收不到键盘消息；改用滚轮消息并复核
                    Write-Log 'WARN' "PageDown 未改变画面（page=$page），改用 WM_MOUSEWHEEL"
                    Send-Wheel -Hwnd $hwnd -Notches 3
                    Save-WindowScreenshot -Hwnd $hwnd -Path $shot | Out-Null
                    if ((Get-ShotHash $prevShot) -eq (Get-ShotHash $shot)) {
                        $stuck++
                        Write-Log 'FAIL' "滚动未生效：截图与上一页完全相同，不能作为深滚动证据（page=$page）"
                        # 连续两页都滚不动：文档已到末尾，继续翻页只会重复报 FAIL，误伤结论。
                        if ($stuck -ge 2) {
                            Write-Log 'INFO' "连续 2 页无变化：判定已到文末（或已达滚动边界），宽度 ${width}px 深滚动结束，共 $page 页"
                            break
                        }
                    } else {
                        $stuck = 0
                    }
                } else {
                    $stuck = 0
                }
                $prevShot = $shot
            }
        }
    }

    if ($Measure) {
        $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if ($p) {
            $ws = [Math]::Round($p.WorkingSet64 / 1MB, 2)
            $pv = [Math]::Round($p.PrivateMemorySize64 / 1MB, 2)
            Write-Log 'METRIC' "启动到窗口=${openMs}ms  WorkingSet=${ws}MB  Private=${pv}MB"
        } else {
            Write-Log 'WARN' '进程已退出，无法采集内存'
        }
    }

    if ($Probe) {
        Start-Sleep -Milliseconds 200
        if (Test-Path -LiteralPath $stderrPath) {
            $probeLines = Get-Content -LiteralPath $stderrPath -ErrorAction SilentlyContinue |
                Where-Object { $_ -match 'BREAK|禁则|violation' }
            Write-Log 'INFO' "探针命中行数：$($probeLines.Count)（原始日志：$stderrPath）"
            foreach ($line in ($probeLines | Select-Object -First 20)) { Write-Log 'PROBE' $line }
        }
    }
} catch {
    Write-Log 'FAIL' "脚本失败：$($_.Exception.Message)"
    $exitCode = 1
} finally {
    if ($procId -ne 0 -and -not $KeepAlive) {
        try {
            $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
            if ($p) {
                Stop-Process -Id $procId -Force
                Write-Log 'INFO' "已结束 PID=$procId"
            }
        } catch {
            Write-Log 'WARN' "结束进程失败：$($_.Exception.Message)"
        }
    }
    if ($null -eq $oldProbe) { Remove-Item Env:VEIL_BREAK_PROBE -ErrorAction SilentlyContinue }
    else { $env:VEIL_BREAK_PROBE = $oldProbe }
    if ($null -eq $oldPerf) { Remove-Item Env:VEIL_PERF_PROBE -ErrorAction SilentlyContinue }
    else { $env:VEIL_PERF_PROBE = $oldPerf }
    Write-Log 'INFO' "证据目录：$ShotDir"
}

if ($exitCode) { exit $exitCode }
