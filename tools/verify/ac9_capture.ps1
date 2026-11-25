# AC9 视觉签名取证：按主题截取 Veil 窗口（证据工具，非 CI 门禁）。
#
# 用法示例：
#   pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Theme light -Region full
#   pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Theme dark -Region menu -Doc .\README.zh-CN.md
#   pwsh -NoProfile -File tools/verify/ac9_capture.ps1 -Help
#
# 主题如何指定（已验证的产品行为，见 crates/veil-ui/src/theme.rs）：
#   - `VEIL_HOME` 非空时，配置读写落到 `<VEIL_HOME>/config.toml`（优先于 %APPDATA%）；
#   - 配置格式为扁平 TOML，主题键是 `mode = "system" | "light" | "dark"`；
#   - 因此本脚本用临时 VEIL_HOME + 预置 config.toml 可靠地指定亮/暗主题，不污染用户配置。
#   - `-Theme auto` 写入 `mode = "system"`：实际生效值由系统外观决定，脚本读取
#     `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`
#     （0 = 深色；读不到时产品自身回退浅色，脚本同样按浅色记录并 WARN）。
#   - 截完图后按像素平均亮度做一次交叉校验；若与声明主题强烈矛盾，按亮度修正主题并 WARN。
#
# 已知近似与限制（诚实优先）：
#   - 只有 full 是整窗。chrome / menu / scrollbar 是「固定几何裁剪」，不是 UI 元素级定位：
#     chrome = 非客户区标题栏 + 30 逻辑像素菜单栏；menu = 客户端顶部 30 逻辑像素；
#     scrollbar = 右侧约 20 逻辑像素竖条。菜单栏/滚动条尺寸取自 chrome 模块常量，
#     布局若变化不会自动跟随，日志会标注为近似。
#   - `-Region modal` 本身不会制造模态；配合 `-SendKeys "^,"` 才能截到设置模态。
#   - `-SendKeys` / `-ClickMenu` 是脚本化合成输入（证据 header/日志如实记录）。产品未实现
#     Alt+F 助记键（实测 Alt+F 会向文档插入 "f"），故主菜单下拉需 `-ClickMenu` 的合成鼠标点击。
#   - PrintWindow 全黑时降级为「临时置顶 + CopyFromScreen」，并 WARN；若降级后仍全黑则
#     保留 `<name>.blank.png` 供诊断并以 exit 1 结束，不把空白图冒充证据。
#
# 失败（找不到 exe / 进程提前退出 / 窗口超时 / 截图全黑且降级失败）一律 exit 1，
# 并打印可读原因。

[CmdletBinding()]
param(
    [string]$Exe = '',
    [ValidateSet('auto', 'light', 'dark')]
    [string]$Theme = 'auto',
    [string]$Doc = '',
    [string]$OutDir = '',
    [ValidateSet('chrome', 'menu', 'modal', 'scrollbar', 'full')]
    [string]$Region = 'full',
    [string]$SendKeys = '',
    [int]$SettleMs = 800,
    [switch]$ClickMenu,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Show-Usage {
    @'
AC9 视觉签名取证：按主题截取 Veil 窗口

用法：
  pwsh -NoProfile -File tools/verify/ac9_capture.ps1 [参数]

参数：
  -Exe <path>        veil.exe 路径（默认：<repo>\target\release\veil.exe）
  -Theme <mode>      auto | light | dark，默认 auto
  -Doc <path>        要打开的 Markdown 文件（默认：README.zh-CN.md 的临时副本）
  -OutDir <dir>      输出目录（默认：<repo>\evidence\verify\ac9）
  -Region <name>     chrome | menu | modal | scrollbar | full，默认 full
  -SendKeys <s>      合成按键串（System.Windows.Forms.SendKeys 语法），例如 "^," = Ctrl+,
  -SettleMs <int>    合成输入后的稳定等待毫秒数，默认 800
  -ClickMenu         合成鼠标点击菜单栏首项（「文件」）以展开主菜单
  -Help              打印本帮助并 exit 0

输出：
  <OutDir>\<YYYYMMDD>-<light|dark>-<region>.png
  <OutDir>\<YYYYMMDD>-<light|dark>-<region>-masked.png（仅 menu / modal：裁掉标题栏）
  <OutDir>\ac9-capture.log

说明：
  - 用隔离的临时 VEIL_HOME 启动，不读写用户真实配置；结束时关闭进程并删除临时目录。
  - light/dark 通过 <VEIL_HOME>\config.toml 的 mode 键预置；auto 用 mode="system"，
    实际主题读 HKCU AppsUseLightTheme（缺失时按产品行为回退浅色），并用像素亮度交叉校验。
  - -SendKeys / -ClickMenu 是脚本化合成输入，用于可复现地打开菜单/模态：
    发送前显式 SetForegroundWindow（置前失败则 WARN 并 exit 1，绝不静默产出错误证据），
    发送后等待 -SettleMs 再截图。证据日志会如实记录实际使用的合成输入。
  - 产品未实现 Alt+F 菜单助记键（实测 Alt+F 会向文档插入 "f" 并标脏），
    因此主菜单下拉需用 -ClickMenu 的可复现鼠标点击；-SendKeys "%f" 不会展开菜单。
  - Region=menu 且使用了合成输入时，裁剪扩展为整窗（以便包含菜单下拉面板）。
  - 除 full 外的区域是固定几何近似裁剪，不是元素级定位。
  - modal 本身不会自动打开模态对话框；请配合 -SendKeys "^," 使用。
  - menu / modal 额外产出去标题栏的 -masked.png（供人工盲评，裁剪像素数写入日志）。
  - 失败一律 exit 1，并打印原因（不会留下冒充证据的空白图）。
'@ | Write-Host
}

if ($Help) {
    Show-Usage
    exit 0
}

if ($SettleMs -lt 0) { throw "-SettleMs 不能为负数：$SettleMs" }

# ---------- 路径与运行期状态 ----------
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'evidence\verify\ac9' }

$script:logPath = ''
$script:proc = $null
$script:tempRoot = ''
$script:stderrPath = ''

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    if ($script:logPath) {
        Add-Content -LiteralPath $script:logPath -Value $line -Encoding utf8
    }
    Write-Host $line
}

# ---------- Win32 ----------
Add-Type -AssemblyName System.Drawing
if (-not ('VeilVerify.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace VeilVerify {
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public struct POINT {
        public int X;
        public int Y;
    }

    public static class Win32 {
        public delegate bool EnumProc(IntPtr hWnd, IntPtr param);

        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr param);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
        [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    }

    public static class K {
        public const uint PW_RENDERFULLCONTENT = 2;
        public const uint SWP_NOSIZE = 0x0001;
        public const uint SWP_NOMOVE = 0x0002;
        public const uint SWP_NOACTIVATE = 0x0010;
        public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
        public const uint MOUSEEVENTF_LEFTUP = 0x0004;
        public const int SW_RESTORE = 9;
        public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
        public static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    }
}
'@
}

# 高 DPI 下不声明感知会让 GetWindowRect 与 CopyFromScreen 拿到不一致的坐标，
# 混用后截图只覆盖窗口左上角的一部分（walkthrough.ps1 已记录该坑）。
[VeilVerify.Win32]::SetProcessDPIAware() | Out-Null

# ---------- 通用工具 ----------
function Stop-VeilProcess {
    param($Process)
    if ($null -eq $Process) { return }
    try {
        if (-not $Process.HasExited) {
            $Process.CloseMainWindow() | Out-Null
            if (-not $Process.WaitForExit(5000)) {
                $Process.Kill()
                $Process.WaitForExit(5000) | Out-Null
            }
        }
        Write-Log 'INFO' "进程已结束：PID=$($Process.Id) ExitCode=$($Process.ExitCode)"
    } catch {
        Write-Log 'WARN' "结束进程失败：$($_.Exception.Message)"
    }
}

function Remove-TempDir {
    param([string]$Path)
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return }
    for ($i = 0; $i -lt 3; $i++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        } catch {
            Start-Sleep -Milliseconds 400
        }
    }
    Write-Log 'WARN' "临时目录未能完全删除（可能有句柄未释放）：$Path"
}

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilVerify.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [VeilVerify.Win32]::IsWindowVisible($h)) { return $true }
        if ([VeilVerify.Win32]::IsIconic($h)) { return $true }
        $r = New-Object VeilVerify.RECT
        [VeilVerify.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [VeilVerify.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [VeilVerify.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
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
        if ($windows.Count -gt 0) {
            return $windows[0]
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    throw "等待主窗口超时（${TimeoutMs}ms）：PID=$($Process.Id) 没有可见顶层窗口（>200x300）"
}

function Add-LogRaw {
    # 证据头行：不经 Write-Log 的时间戳前缀，保持 `# key = value` 原样可读。
    param([string]$Line)
    if ($script:logPath) {
        Add-Content -LiteralPath $script:logPath -Value $Line -Encoding utf8
    }
    Write-Host $Line
}

function Invoke-SyntheticInput {
    # 可复现合成输入：先把目标窗口置前（失败即非零退出），再发送按键/点击菜单栏首项。
    param([IntPtr]$Hwnd, [string]$Keys, [int]$Settle, [double]$Scale, [switch]$ClickMenu)
    [VeilVerify.Win32]::ShowWindow($Hwnd, [VeilVerify.K]::SW_RESTORE) | Out-Null
    $foreground = $false
    $fg = [IntPtr]::Zero
    for ($i = 0; $i -lt 3 -and -not $foreground; $i++) {
        [VeilVerify.Win32]::SetForegroundWindow($Hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        $fg = [VeilVerify.Win32]::GetForegroundWindow()
        $foreground = ($fg -eq $Hwnd)
    }
    if (-not $foreground) {
        Write-Log 'WARN' "置前失败：GetForegroundWindow=$fg 不是目标 HWND=$Hwnd；按键会打到别的窗口，拒绝对此取证"
        throw "SetForegroundWindow 失败（目标 HWND=$Hwnd，实际前台=$fg）"
    }
    Write-Log 'INFO' "窗口已置前：HWND=$Hwnd"

    if ($ClickMenu) {
        # 菜单栏首项「文件」内部坐标：x 约 20 逻辑像素、y 约 15 逻辑像素（MENU_H=30）。
        $pt = New-Object VeilVerify.POINT
        $pt.X = [int][Math]::Round(20 * $Scale)
        $pt.Y = [int][Math]::Round(15 * $Scale)
        [VeilVerify.Win32]::ClientToScreen($Hwnd, [ref]$pt) | Out-Null
        [VeilVerify.Win32]::SetCursorPos($pt.X, $pt.Y) | Out-Null
        Start-Sleep -Milliseconds 120
        [VeilVerify.Win32]::mouse_event([VeilVerify.K]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 60
        [VeilVerify.Win32]::mouse_event([VeilVerify.K]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
        Write-Log 'INFO' "已合成点击菜单栏首项：屏幕坐标 $($pt.X),$($pt.Y)"
    }

    if ($Keys) {
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.SendKeys]::SendWait($Keys)
        Write-Log 'INFO' "已合成按键：SendKeys '$Keys'"
    }

    if ($Settle -gt 0) {
        Write-Log 'INFO' "等待合成输入稳定 ${Settle}ms"
        Start-Sleep -Milliseconds $Settle
    }
}

function Get-SystemThemeName {
    # 与产品 platform/theme.rs 同源：AppsUseLightTheme=0 即深色；缺失/失败返回 $null。
    try {
        $item = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -Name 'AppsUseLightTheme' -ErrorAction Stop
        if ($null -eq $item -or $null -eq $item.AppsUseLightTheme) { return $null }
        if ([int]$item.AppsUseLightTheme -eq 0) { return 'dark' }
        return 'light'
    } catch {
        return $null
    }
}

function Test-BlankBitmap {
    # 采样 16x10 点：颜色种类不超过 2 视为空白（与 walkthrough.ps1 的判定一致）。
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

function Invoke-WindowCapture {
    param([IntPtr]$Hwnd)
    $wr = New-Object VeilVerify.RECT
    [VeilVerify.Win32]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) {
        return [pscustomobject]@{ Bitmap = $null; Ok = $false; Blank = $true; Method = 'none'; Degraded = $false; Reason = "窗口尺寸无效 ${w}x${h}" }
    }

    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $printed = $false
    try {
        $printed = [VeilVerify.Win32]::PrintWindow($Hwnd, $hdc, [VeilVerify.K]::PW_RENDERFULLCONTENT)
    } finally {
        $g.ReleaseHdc($hdc)
        $g.Dispose()
    }

    $blank = if ($printed) { Test-BlankBitmap -Bitmap $bmp } else { $true }
    $method = 'PrintWindow'
    $degraded = $false

    if (-not $printed -or $blank) {
        # 自绘窗口 PrintWindow 常返回全黑：临时置顶后屏幕捕获，随后恢复 z-order。
        # 不调用 SetForegroundWindow，避免抢走用户焦点。
        $flags = ([VeilVerify.K]::SWP_NOMOVE -bor [VeilVerify.K]::SWP_NOSIZE -bor [VeilVerify.K]::SWP_NOACTIVATE)
        [VeilVerify.Win32]::SetWindowPos($Hwnd, [VeilVerify.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 250
            $g2 = [System.Drawing.Graphics]::FromImage($bmp)
            try {
                $g2.CopyFromScreen($wr.Left, $wr.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
            } finally {
                $g2.Dispose()
            }
        } catch {
            $bmp.Dispose()
            return [pscustomobject]@{ Bitmap = $null; Ok = $false; Blank = $true; Method = 'none'; Degraded = $true; Reason = "屏幕捕获失败：$($_.Exception.Message)" }
        } finally {
            [VeilVerify.Win32]::SetWindowPos($Hwnd, [VeilVerify.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        }
        $method = 'CopyFromScreen'
        $degraded = $true
        $blank = Test-BlankBitmap -Bitmap $bmp
        Write-Log 'WARN' "PrintWindow 无效（printed=$printed），已降级为置顶屏幕捕获：blank=$blank"
    }

    if ($blank) {
        # 保留空白图供诊断，但由调用方改名保存为 .blank.png，绝不冒充正式证据。
        return [pscustomobject]@{ Bitmap = $bmp; Ok = $false; Blank = $true; Method = $method; Degraded = $degraded; Reason = '截图全黑/纯色，降级后仍然空白' }
    }
    return [pscustomobject]@{ Bitmap = $bmp; Ok = $true; Blank = $false; Method = $method; Degraded = $degraded; Reason = '' }
}

function Get-RegionRect {
    param(
        [int]$WinW, [int]$WinH,
        [int]$ClientLeft, [int]$ClientTop,
        [int]$ClientW, [int]$ClientH,
        [double]$Scale,
        [string]$Region
    )
    # chrome 模块常量：MENU_H = 30 逻辑像素、SCROLL_W = 12 逻辑像素。
    $menuH = [int][Math]::Max(1, [Math]::Round(30 * $Scale))
    switch ($Region) {
        'full' {
            return [System.Drawing.Rectangle]::new(0, 0, $WinW, $WinH)
        }
        'chrome' {
            $height = [Math]::Min($WinH, $ClientTop + $menuH + [int][Math]::Round(8 * $Scale))
            return [System.Drawing.Rectangle]::new(0, 0, $WinW, [Math]::Max(1, $height))
        }
        'menu' {
            $y = [Math]::Max(0, $ClientTop)
            $height = [Math]::Max(1, [Math]::Min($menuH, $WinH - $y))
            return [System.Drawing.Rectangle]::new(0, $y, $WinW, $height)
        }
        'scrollbar' {
            $width = [Math]::Max(1, [Math]::Min($WinW, [int][Math]::Round((12 + 8) * $Scale)))
            $y = [Math]::Max(0, $ClientTop)
            $height = [Math]::Max(1, [Math]::Min($ClientH, $WinH - $y))
            return [System.Drawing.Rectangle]::new($WinW - $width, $y, $width, $height)
        }
        'modal' {
            # 脚本不制造模态：取整窗基线，由调用方在日志中 WARN。
            return [System.Drawing.Rectangle]::new(0, 0, $WinW, $WinH)
        }
        default {
            throw "未知区域：$Region"
        }
    }
}

# ---------- 主流程 ----------
$exitCode = 0
$regionRect = $null
try {
    if (-not (Test-Path -LiteralPath $Exe)) {
        throw "找不到 exe：$Exe（请先 cargo build --release -b veil，或用 -Exe 指定）"
    }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path

    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    $OutDir = (Resolve-Path -LiteralPath $OutDir).Path
    $script:logPath = Join-Path $OutDir 'ac9-capture.log'
    Write-Log 'INFO' "AC9 开始：Exe=$Exe Region=$Region Theme=$Theme OutDir=$OutDir"

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $script:tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-verify-ac9-{0}-{1}" -f $stamp, $PID)
    $veilHome = Join-Path $script:tempRoot 'home'
    $appData = Join-Path $script:tempRoot 'appdata'
    $localAppData = Join-Path $script:tempRoot 'localappdata'
    $tempDir = Join-Path $script:tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null
    Write-Log 'INFO' "隔离根目录：$($script:tempRoot)"

    # 文档：默认复制 README.zh-CN.md，避免直接改动仓库内文件。
    if (-not $Doc) {
        $readme = Join-Path $repoRoot 'README.zh-CN.md'
        if (-not (Test-Path -LiteralPath $readme)) {
            throw "未提供 -Doc，且仓库根目录缺少 README.zh-CN.md：$readme"
        }
        $Doc = Join-Path $script:tempRoot 'ac9-doc.md'
        Copy-Item -LiteralPath $readme -Destination $Doc -Force
        Write-Log 'INFO' "未指定 -Doc，使用 README.zh-CN.md 的临时副本：$Doc"
    } else {
        if (-not (Test-Path -LiteralPath $Doc)) { throw "找不到 -Doc 文件：$Doc" }
        $Doc = (Resolve-Path -LiteralPath $Doc).Path
        Write-Log 'INFO' "使用指定文档：$Doc"
    }

    # 主题：light/dark 直接预置；auto 写 system，实际值随后按系统外观推导。
    $configMode = if ($Theme -eq 'auto') { 'system' } else { $Theme }
    $configPath = Join-Path $veilHome 'config.toml'
    [IO.File]::WriteAllText($configPath, "mode = `"$configMode`"`n", [Text.UTF8Encoding]::new($false))
    Write-Log 'INFO' "预置配置：$configPath（mode=$configMode）"

    if ($Theme -eq 'auto') {
        $systemTheme = Get-SystemThemeName
        if ($null -eq $systemTheme) {
            $actualTheme = 'light'
            Write-Log 'WARN' '无法读取 HKCU AppsUseLightTheme；按产品回退行为记录为 light（请人工核对截图）'
        } else {
            $actualTheme = $systemTheme
        }
    } else {
        $actualTheme = $Theme
    }
    Write-Log 'INFO' "主题声明=$Theme 实际记录=$actualTheme"

    # 启动：VEIL_HOME/APPDATA/TEMP 全部隔离，避免读写用户真实配置与临时文件。
    $script:stderrPath = Join-Path $script:tempRoot 'stderr.log'
    $stdoutPath = Join-Path $script:tempRoot 'stdout.log'
    $startArgs = @{
        FilePath = $Exe
        ArgumentList = @($Doc)
        PassThru = $true
        RedirectStandardOutput = $stdoutPath
        RedirectStandardError = $script:stderrPath
        Environment = @{
            VEIL_HOME = $veilHome
            APPDATA = $appData
            LOCALAPPDATA = $localAppData
            TEMP = $tempDir
            TMP = $tempDir
        }
    }
    $script:proc = Start-Process @startArgs
    Write-Log 'INFO' "已启动：PID=$($script:proc.Id) 文档=$Doc"

    $window = Wait-MainWindow -Process $script:proc -TimeoutMs 20000
    Write-Log 'INFO' "主窗口出现：HWND=$($window.Hwnd) 标题=$($window.Title)"
    Start-Sleep -Milliseconds 1000

    $hwnd = [IntPtr]$window.Hwnd
    $wr = New-Object VeilVerify.RECT
    [VeilVerify.Win32]::GetWindowRect($hwnd, [ref]$wr) | Out-Null
    $cr = New-Object VeilVerify.RECT
    [VeilVerify.Win32]::GetClientRect($hwnd, [ref]$cr) | Out-Null
    $origin = New-Object VeilVerify.POINT
    [VeilVerify.Win32]::ClientToScreen($hwnd, [ref]$origin) | Out-Null
    $winW = $wr.Right - $wr.Left
    $winH = $wr.Bottom - $wr.Top
    $clientLeft = $origin.X - $wr.Left
    $clientTop = $origin.Y - $wr.Top
    $clientW = $cr.Right - $cr.Left
    $clientH = $cr.Bottom - $cr.Top
    $dpi = [uint32][VeilVerify.Win32]::GetDpiForWindow($hwnd)
    if ($dpi -eq 0) { $dpi = 96 }
    $scale = $dpi / 96.0
    Write-Log 'INFO' "窗口几何：${winW}x${winH}，客户区 ${clientW}x${clientH}，客户区偏移 ${clientLeft},${clientTop}，DPI=$dpi（scale=$([Math]::Round($scale, 3))）"

    # 证据头：如实记录合成输入（未使用写 (none)）。
    $syntheticUsed = [bool]($SendKeys -or $ClickMenu)
    Add-LogRaw ("# send_keys = " + $(if ($SendKeys) { "$SendKeys (synthetic input)" } else { '(none)' }))
    Add-LogRaw ("# click_menu = " + $(if ($ClickMenu) { 'yes (synthetic mouse click on menu bar item 0)' } else { '(none)' }))
    Add-LogRaw "# settle_ms = $SettleMs"

    if ($syntheticUsed) {
        Write-Log 'INFO' '合成输入：脚本化合成输入（SendKeys / 菜单点击），证据由此可复现；非人工操作'
        Invoke-SyntheticInput -Hwnd $hwnd -Keys $SendKeys -Settle $SettleMs -Scale $scale -ClickMenu:$ClickMenu
    }

    switch ($Region) {
        'full' { Write-Log 'INFO' '区域=full：整窗' }
        'chrome' { Write-Log 'INFO' '区域=chrome：近似裁剪（非客户区标题栏 + 30 逻辑像素菜单栏），非元素级定位' }
        'menu' {
            if ($syntheticUsed) {
                Write-Log 'INFO' '区域=menu：合成输入可能展开下拉，裁剪扩展为整窗（含菜单栏 + 下拉面板），非元素级定位'
            } else {
                Write-Log 'INFO' '区域=menu：近似裁剪（客户端顶部 30 逻辑像素菜单栏），非元素级定位'
            }
        }
        'scrollbar' { Write-Log 'INFO' '区域=scrollbar：近似裁剪（右侧约 20 逻辑像素竖条），非元素级定位' }
        'modal' {
            if ($SendKeys) {
                Write-Log 'INFO' "区域=modal：已用合成按键 '$SendKeys' 打开模态；整窗截图（含标题栏）"
            } else {
                Write-Log 'WARN' '区域=modal：脚本不会自动制造未保存状态，本图为整窗基线（近似），不是模态对话框截图'
            }
        }
    }

    $capture = Invoke-WindowCapture -Hwnd $hwnd
    if ($null -eq $capture.Bitmap -or -not $capture.Ok) {
        $reason = if ($capture.Reason) { $capture.Reason } else { '未知错误' }
        if ($null -ne $capture.Bitmap) {
            $blankPath = Join-Path $OutDir ('{0}-{1}-{2}.blank.png' -f (Get-Date -Format 'yyyyMMdd'), $actualTheme, $Region)
            $capture.Bitmap.Save($blankPath, [System.Drawing.Imaging.ImageFormat]::Png)
            $capture.Bitmap.Dispose()
            Write-Log 'WARN' "空白截图已保留用于诊断：$blankPath"
        }
        throw "截图失败：$reason"
    }

    Write-Log 'INFO' "截图完成：方法=$($capture.Method) 降级=$($capture.Degraded)"

    # 主题交叉校验：只在强信号矛盾时修正，避免把内容亮度误判成主题。
    $luminance = Get-AverageLuminance -Bitmap $capture.Bitmap
    if ($luminance -ge 0) {
        if ($actualTheme -eq 'light' -and $luminance -lt 60) {
            Write-Log 'WARN' ("平均亮度 {0:N1} 明显偏暗，与 light 矛盾；按像素证据修正实际主题为 dark" -f $luminance)
            $actualTheme = 'dark'
        } elseif ($actualTheme -eq 'dark' -and $luminance -gt 200) {
            Write-Log 'WARN' ("平均亮度 {0:N1} 明显偏亮，与 dark 矛盾；按像素证据修正实际主题为 light" -f $luminance)
            $actualTheme = 'light'
        } else {
            Write-Log 'INFO' ("亮度校验通过：平均亮度 {0:N1}，主题 {1}" -f $luminance, $actualTheme)
        }
    }

    $regionRect = Get-RegionRect -WinW $winW -WinH $winH -ClientLeft $clientLeft -ClientTop $clientTop `
        -ClientW $clientW -ClientH $clientH -Scale $scale -Region $Region
    if ($Region -eq 'menu' -and $syntheticUsed) {
        # 展开的菜单下拉在客户区内、但远低于菜单栏条带：整窗裁剪才包含下拉面板。
        $regionRect = [System.Drawing.Rectangle]::new(0, 0, $winW, $winH)
    }

    $final = $null
    if ($regionRect.X -eq 0 -and $regionRect.Y -eq 0 -and $regionRect.Width -eq $winW -and $regionRect.Height -eq $winH) {
        $final = $capture.Bitmap
    } else {
        $final = New-Object System.Drawing.Bitmap($regionRect.Width, $regionRect.Height)
        $g3 = [System.Drawing.Graphics]::FromImage($final)
        try {
            $dest = [System.Drawing.Rectangle]::new(0, 0, $regionRect.Width, $regionRect.Height)
            $g3.DrawImage($capture.Bitmap, $dest, $regionRect, [System.Drawing.GraphicsUnit]::Pixel)
        } finally {
            $g3.Dispose()
        }
        $capture.Bitmap.Dispose()
    }

    $fileName = '{0}-{1}-{2}.png' -f (Get-Date -Format 'yyyyMMdd'), $actualTheme, $Region
    $outPath = Join-Path $OutDir $fileName
    if (Test-Path -LiteralPath $outPath) {
        Write-Log 'WARN' "同名截图已存在，将覆盖：$outPath"
    }
    try {
        $final.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)

        # 去标题栏遮罩版（仅 menu / modal，人工盲评用）：按实测非客户区顶部高度裁剪。
        if (($Region -eq 'menu' -or $Region -eq 'modal') -and $final.Height -gt 200) {
            $maskTop = [int]$clientTop
            if ($maskTop -le 0 -or $maskTop -ge [int]($final.Height / 3)) {
                $maskTop = [int][Math]::Round(50 * $scale)
                Write-Log 'WARN' "非客户区顶部高度异常，遮罩裁剪改用保守值 ${maskTop}px"
            }
            if ($maskTop -gt 0 -and ($final.Height - $maskTop) -gt 100) {
                $masked = New-Object System.Drawing.Bitmap($final.Width, ($final.Height - $maskTop))
                $gm = [System.Drawing.Graphics]::FromImage($masked)
                try {
                    $srcRect = [System.Drawing.Rectangle]::new(0, $maskTop, $final.Width, ($final.Height - $maskTop))
                    $dstRect = [System.Drawing.Rectangle]::new(0, 0, $masked.Width, $masked.Height)
                    $gm.DrawImage($final, $dstRect, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
                } finally {
                    $gm.Dispose()
                }
                $maskedName = [IO.Path]::GetFileNameWithoutExtension($fileName) + '-masked.png'
                $maskedPath = Join-Path $OutDir $maskedName
                try {
                    $masked.Save($maskedPath, [System.Drawing.Imaging.ImageFormat]::Png)
                } finally {
                    $masked.Dispose()
                }
                Add-LogRaw "# mask_crop_top = ${maskTop}px (measured non-client top at scale=$([Math]::Round($scale, 3)))"
                Write-Log 'PASS' "去标题栏遮罩版已保存：$maskedPath（顶部裁剪 ${maskTop}px，尺寸 $($final.Width)x$($final.Height - $maskTop)）"
                Write-Host "输出（遮罩）：$maskedPath"
            } else {
                Write-Log 'WARN' '遮罩版跳过：裁剪后高度不足'
            }
        }
    } finally {
        $final.Dispose()
    }

    $fileInfo = Get-Item -LiteralPath $outPath
    if ($fileInfo.Length -le 0) {
        throw "截图文件写入后为空：$outPath"
    }
    Write-Log 'PASS' "截图已保存：$outPath（方法=$($capture.Method)，降级=$($capture.Degraded)，裁剪=$($regionRect.Width)x$($regionRect.Height)，字节=$($fileInfo.Length)）"
    Write-Host "输出：$outPath"
    $exitCode = 0
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    if ($script:logPath) {
        Add-Content -LiteralPath $script:logPath -Value ('[{0}] FAIL  {1}' -f (Get-Date -Format 'HH:mm:ss.fff'), $_.Exception.Message) -Encoding utf8
    }
    $exitCode = 1
} finally {
    # stderr 取证：先读后删临时目录。
    if ($script:stderrPath -and (Test-Path -LiteralPath $script:stderrPath)) {
        $stderrInfo = Get-Item -LiteralPath $script:stderrPath
        if ($stderrInfo.Length -gt 0) {
            $tail = @(Get-Content -LiteralPath $script:stderrPath -Tail 20)
            Write-Log 'INFO' "stderr 输出（最后 $($tail.Count) 行）：$($tail -join ' | ')"
        }
    }
    Stop-VeilProcess -Process $script:proc
    Remove-TempDir -Path $script:tempRoot
}

exit $exitCode
