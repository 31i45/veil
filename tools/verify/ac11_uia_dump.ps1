# AC11 屏幕阅读器取证：dump Veil 主窗口的 UIA 无障碍树。
#
# 用法示例：
#   pwsh -NoProfile -File tools/verify/ac11_uia_dump.ps1 -Scenario menu
#   pwsh -NoProfile -File tools/verify/ac11_uia_dump.ps1 -Scenario modal -WaitMs 20000
#   pwsh -NoProfile -File tools/verify/ac11_uia_dump.ps1 -Help
#
# 前置条件（脚本会在树为空时明确报错）：
#   产品必须以无障碍 feature 构建：cargo build --release --bin veil --features accessibility
#   未启用时 AccessKit/UIA 提供者不存在，除根节点外通常没有子节点。
#
# 使用的技术：
#   - 只用 Windows 内置的 UIAutomationClient / UIAutomationTypes（不引入第三方包）；
#   - [AutomationElement]::FromHandle($hwnd) 取主窗口根元素；
#   - 用 TreeWalker.ControlViewWalker 递归遍历（最大深度 12、最大节点 5000，防死循环）；
#   - 每行：缩进 + ControlType.ProgrammaticName | Name | IsEnabled | IsOffscreen | BoundingRectangle。
#
# 场景限制（诚实优先）：
#   - menu：先稳定等待，再做可复现的合成输入（-ClickMenu 点击菜单栏首项；-SendKeys 按键），
#     然后 dump。注意：产品未实现 Alt+F 助记键（实测 Alt+F 只会向文档插入 "f"），且主菜单
#     只在展开时才把 Menu/MenuItem 暴露到语义树，所以菜单取证必须 -ClickMenu。
#   - modal：配合 -SendKeys "^," 可复现地打开「设置」模态，再 dump 模态打开时的树；
#     不提供合成输入时只 dump 初始树。
#   - 结束时关闭进程并删除临时 VEIL_HOME，不污染用户配置。
#
# 失败（找不到 exe / 窗口超时 / 进程在 UIA 调用前退出 / UIA 树为空）一律 exit 1，并给出可读原因。
#   若无障碍构建在启动时 panic（例如 AccessKit adapter 创建时机错误），脚本会报告进程退出码与 stderr，
#   不会把「进程已崩溃」误报成「UIA 树为空」。

[CmdletBinding()]
param(
    [string]$Exe = '',
    [ValidateSet('menu', 'modal')]
    [string]$Scenario = 'menu',
    [string]$OutFile = '',
    [int]$WaitMs = 2000,
    [string]$SendKeys = '',
    [int]$SettleMs = 800,
    [switch]$ClickMenu,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Show-Usage {
    @'
AC11 屏幕阅读器取证：dump Veil 主窗口的 UIA 无障碍树

用法：
  pwsh -NoProfile -File tools/verify/ac11_uia_dump.ps1 [参数]

参数：
  -Exe <path>      veil.exe 路径（默认：<repo>\target\release\veil.exe）
  -Scenario <name> menu | modal，默认 menu
  -OutFile <path>  输出文本路径（默认：<repo>\evidence\verify\ac11\uia-tree-<scenario>.txt）
  -WaitMs <int>    窗口出现后的稳定等待毫秒数，默认 2000
  -SendKeys <s>    稳定等待后发送的合成按键串（SendKeys 语法），例如 "^," = Ctrl+, 打开设置模态
  -SettleMs <int>  合成输入后的稳定等待毫秒数，默认 800
  -ClickMenu       合成鼠标点击菜单栏首项（「文件」）以展开主菜单
  -Help            打印本帮助并 exit 0

输出：
  每行：缩进 + ControlType.ProgrammaticName | "Name" | IsEnabled | IsOffscreen | x,y,w,h
  文件为 UTF-8（无 BOM），开头含场景/句柄/节点数等摘要注释行。

说明：
  - 需要产品以 --features accessibility 构建，否则 UIA 树为空，脚本 exit 1 并提示构建命令。
  - -SendKeys / -ClickMenu 是脚本化合成输入，用于可复现地打开菜单/模态。发送前显式
    SetForegroundWindow；置前失败则 WARN 并 exit 1，不产出错误证据。header 如实记录。
  - 产品未实现 Alt+F 菜单助记键（实测 Alt+F 向文档插入 "f" 并标脏，UIA 树不变）；
    主菜单只在展开时才把 Menu/MenuItem 暴露到语义树，故菜单取证需 -ClickMenu。
  - modal 场景配合 -SendKeys "^," 打开设置模态；未提供时只 dump 初始树。
  - 使用临时 VEIL_HOME 隔离启动，结束时关闭进程并清理临时目录。
'@ | Write-Host
}

if ($Help) {
    Show-Usage
    exit 0
}

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\release\veil.exe' }
if (-not $OutFile) { $OutFile = Join-Path $repoRoot ("evidence\verify\ac11\uia-tree-{0}.txt" -f $Scenario) }
if ($WaitMs -lt 0) { throw "-WaitMs 不能为负数：$WaitMs" }
if ($SettleMs -lt 0) { throw "-SettleMs 不能为负数：$SettleMs" }

$script:proc = $null
$script:tempRoot = ''
$script:stderrPath = ''
$script:outDir = ''

function Write-Log {
    param([string]$Level, [string]$Message)
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Level, $Message
    Write-Host $line
}

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

# ---------- Windows 无障碍 API（系统内置） ----------
try {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
} catch {
    throw "无法加载 Windows UIAutomation 程序集：$($_.Exception.Message)（需在 Windows 交互桌面会话中运行）"
}
if (-not ('System.Windows.Automation.AutomationElement' -as [type])) {
    throw 'UIAutomationClient 已加载但找不到 AutomationElement 类型'
}

# ---------- Win32（仅窗口枚举与等待） ----------
if (-not ('VeilVerifyUia.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace VeilVerifyUia {
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
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    }

    public static class K {
        public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
        public const uint MOUSEEVENTF_LEFTUP = 0x0004;
        public const int SW_RESTORE = 9;
    }
}
'@
}

[VeilVerifyUia.Win32]::SetProcessDPIAware() | Out-Null

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = New-Object System.Collections.Generic.List[object]
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilVerifyUia.Win32]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [VeilVerifyUia.Win32]::IsWindowVisible($h)) { return $true }
        if ([VeilVerifyUia.Win32]::IsIconic($h)) { return $true }
        $r = New-Object VeilVerifyUia.RECT
        [VeilVerifyUia.Win32]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [VeilVerifyUia.Win32]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [VeilVerifyUia.Win32]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
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

function Invoke-SyntheticInput {
    # 可复现合成输入：先显式置前（失败即非零退出），再发送按键 / 点击菜单栏首项。
    param([IntPtr]$Hwnd, [string]$Keys, [int]$Settle, [switch]$ClickMenu)
    [VeilVerifyUia.Win32]::ShowWindow($Hwnd, [VeilVerifyUia.K]::SW_RESTORE) | Out-Null
    $foreground = $false
    $fg = [IntPtr]::Zero
    for ($i = 0; $i -lt 3 -and -not $foreground; $i++) {
        [VeilVerifyUia.Win32]::SetForegroundWindow($Hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        $fg = [VeilVerifyUia.Win32]::GetForegroundWindow()
        $foreground = ($fg -eq $Hwnd)
    }
    if (-not $foreground) {
        Write-Log 'WARN' "置前失败：GetForegroundWindow=$fg 不是目标 HWND=$Hwnd；按键会打到别的窗口，拒绝对此取证"
        throw "SetForegroundWindow 失败（目标 HWND=$Hwnd，实际前台=$fg）"
    }
    Write-Log 'INFO' "窗口已置前：HWND=$Hwnd"

    if ($ClickMenu) {
        $dpi = [uint32][VeilVerifyUia.Win32]::GetDpiForWindow($Hwnd)
        if ($dpi -eq 0) { $dpi = 96 }
        $scale = $dpi / 96.0
        # 菜单栏首项「文件」内部坐标：x 约 20 逻辑像素、y 约 15 逻辑像素（MENU_H=30）。
        $pt = New-Object VeilVerifyUia.POINT
        $pt.X = [int][Math]::Round(20 * $scale)
        $pt.Y = [int][Math]::Round(15 * $scale)
        [VeilVerifyUia.Win32]::ClientToScreen($Hwnd, [ref]$pt) | Out-Null
        [VeilVerifyUia.Win32]::SetCursorPos($pt.X, $pt.Y) | Out-Null
        Start-Sleep -Milliseconds 120
        [VeilVerifyUia.Win32]::mouse_event([VeilVerifyUia.K]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 60
        [VeilVerifyUia.Win32]::mouse_event([VeilVerifyUia.K]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
        Write-Log 'INFO' "已合成点击菜单栏首项：屏幕坐标 $($pt.X),$($pt.Y)（DPI=$dpi）"
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

# ---------- UIA 遍历 ----------
$script:maxDepth = 12
$script:maxNodes = 5000
$script:nodes = 0
$script:menuItems = 0
$script:truncated = $false
$script:depthLimited = $false
$script:lines = New-Object System.Collections.Generic.List[string]
$script:walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker

function Get-ElementText {
    param($Element, [int]$Depth)
    $indent = ' ' * ($Depth * 2)
    try {
        $current = $Element.Current
        $typeName = $current.ControlType.ProgrammaticName
        if (-not $typeName) { $typeName = '<unknown>' }
        $name = $current.Name
        if ($null -eq $name) { $name = '' }
        $name = $name -replace "[\r\n\t]+", ' '
        $name = $name.Replace('"', "'")
        if ($name.Length -gt 120) { $name = $name.Substring(0, 120) + '...' }
        $enabled = $current.IsEnabled
        $offscreen = $current.IsOffscreen
        $rect = $current.BoundingRectangle
        if ($rect.IsEmpty) {
            $rectText = 'empty'
        } else {
            $rectText = '{0},{1},{2},{3}' -f [int]$rect.X, [int]$rect.Y, [int]$rect.Width, [int]$rect.Height
        }
        if ($typeName -eq 'ControlType.MenuItem') { $script:menuItems++ }
        return '{0}{1} | "{2}" | {3} | {4} | {5}' -f $indent, $typeName, $name, $enabled, $offscreen, $rectText
    } catch {
        return ('{0}<读取属性失败：{1}>' -f $indent, $_.Exception.Message)
    }
}

function Visit-Element {
    param($Element, [int]$Depth)
    if ($script:nodes -ge $script:maxNodes) {
        $script:truncated = $true
        return
    }
    $script:nodes++
    $script:lines.Add((Get-ElementText -Element $Element -Depth $Depth))

    if ($Depth -ge $script:maxDepth) {
        $script:depthLimited = $true
        $script:lines.Add((' ' * (($Depth + 1) * 2)) + '<达到深度上限 12，子树未展开>')
        return
    }

    $child = $null
    try {
        $child = $script:walker.GetFirstChild($Element)
    } catch {
        $script:lines.Add((' ' * (($Depth + 1) * 2)) + "<枚举子节点失败：$($_.Exception.Message)>")
        return
    }
    while ($null -ne $child) {
        Visit-Element -Element $child -Depth ($Depth + 1)
        if ($script:nodes -ge $script:maxNodes) {
            $script:truncated = $true
            return
        }
        try {
            $child = $script:walker.GetNextSibling($child)
        } catch {
            $child = $null
        }
    }
}

# ---------- 主流程 ----------
$exitCode = 0
try {
    if (-not (Test-Path -LiteralPath $Exe)) {
        throw "找不到 exe：$Exe（Veil 请先 cargo build --release -b veil --features accessibility；也可用 -Exe 指向其它 GUI 程序做脚本自检）"
    }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path

    $outDir = Split-Path -Parent $OutFile
    if (-not $outDir) { $outDir = '.' }
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
    $script:outDir = (Resolve-Path -LiteralPath $outDir).Path
    $OutFile = [IO.Path]::GetFullPath($OutFile)
    Write-Log 'INFO' "AC11 开始：Exe=$Exe Scenario=$Scenario WaitMs=$WaitMs OutFile=$OutFile"

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $script:tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-verify-ac11-{0}-{1}" -f $stamp, $PID)
    $veilHome = Join-Path $script:tempRoot 'home'
    $appData = Join-Path $script:tempRoot 'appdata'
    $localAppData = Join-Path $script:tempRoot 'localappdata'
    $tempDir = Join-Path $script:tempRoot 'temp'
    New-Item -ItemType Directory -Path $veilHome, $appData, $localAppData, $tempDir -Force | Out-Null
    Write-Log 'INFO' "隔离根目录：$($script:tempRoot)"

    # modal 场景便于人工制造未保存状态：打开 README 的临时副本（不改仓库文件）。
    $launchArgs = @()
    if ($Scenario -eq 'modal') {
        $readme = Join-Path $repoRoot 'README.zh-CN.md'
        if (Test-Path -LiteralPath $readme) {
            $docCopy = Join-Path $script:tempRoot 'uia-doc.md'
            Copy-Item -LiteralPath $readme -Destination $docCopy -Force
            $launchArgs = @($docCopy)
            Write-Log 'INFO' "modal 场景：已准备临时文档副本便于人工编辑：$docCopy"
        } else {
            Write-Log 'WARN' "modal 场景：缺少 README.zh-CN.md，将以空窗口启动；人工需先新建或输入内容"
        }
    }

    $script:stderrPath = Join-Path $script:tempRoot 'stderr.log'
    $stdoutPath = Join-Path $script:tempRoot 'stdout.log'
    $startArgs = @{
        FilePath = $Exe
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
    if ($launchArgs.Count -gt 0) { $startArgs['ArgumentList'] = $launchArgs }
    $script:proc = Start-Process @startArgs
    Write-Log 'INFO' "已启动：PID=$($script:proc.Id)"

    $window = Wait-MainWindow -Process $script:proc -TimeoutMs 20000
    $hwnd = [IntPtr]$window.Hwnd
    Write-Log 'INFO' "主窗口出现：HWND=$hwnd 标题=$($window.Title)"

    if ($WaitMs -gt 0) {
        Write-Log 'INFO' "稳定等待 ${WaitMs}ms"
        Start-Sleep -Milliseconds $WaitMs
    }

    # 合成输入：可复现地打开菜单/模态，必须在窗口就绪之后、dump 之前。
    if ($SendKeys -or $ClickMenu) {
        Write-Log 'INFO' '合成输入：脚本化合成输入（SendKeys / 菜单点击），证据由此可复现；非人工操作'
        Invoke-SyntheticInput -Hwnd $hwnd -Keys $SendKeys -Settle $SettleMs -ClickMenu:$ClickMenu
    }

    # 无障碍构建曾在启动时 panic（AccessKit adapter 必须在窗口首次显示前创建），
    # 窗口出现后进程可能立刻退出；必须在这里如实报告，而不是让 FromHandle 抛出晦涩错误。
    $script:proc.Refresh()
    if ($script:proc.HasExited) {
        $tail = if (Test-Path -LiteralPath $script:stderrPath) { @(Get-Content -LiteralPath $script:stderrPath -Tail 5) -join ' | ' } else { '(无 stderr)' }
        throw "进程在 UIA 调用前退出：PID=$($script:proc.Id) ExitCode=$($script:proc.ExitCode)，stderr=$tail"
    }

    $nonRoot = 0
    $root = $null
    $attempt = 0
    do {
        $attempt++
        $script:proc.Refresh()
        if ($script:proc.HasExited) {
            $tail = if (Test-Path -LiteralPath $script:stderrPath) { @(Get-Content -LiteralPath $script:stderrPath -Tail 5) -join ' | ' } else { '(无 stderr)' }
            throw "进程在重试期间退出：PID=$($script:proc.Id) ExitCode=$($script:proc.ExitCode)，stderr=$tail"
        }
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
        if ($null -eq $root) {
            throw "FromHandle 返回空：HWND=$hwnd（窗口可能已关闭或 UIA 提供者不可用）"
        }
        $script:lines = New-Object System.Collections.Generic.List[string]
        $script:nodes = 0
        $script:menuItems = 0
        $script:truncated = $false
        $script:depthLimited = $false
        Visit-Element -Element $root -Depth 0
        $nonRoot = $script:nodes - 1
        if ($nonRoot -ge 2) { break }
        if ($attempt -lt 3) {
            Write-Log 'WARN' "UIA 树节点不足（除根节点外 $nonRoot 个），1.5s 后重试（第 $attempt 次）"
            Start-Sleep -Milliseconds 1500
        }
    } while ($attempt -lt 3)

    $generated = (Get-Date).ToString('o')
    $sendKeysHeader = if ($SendKeys) { "$SendKeys (synthetic input)" } else { '(none)' }
    $clickMenuHeader = if ($ClickMenu) { 'yes (synthetic mouse click on menu bar item 0)' } else { '(none)' }
    $header = @(
        '# Veil UIA tree dump',
        "# scenario = $Scenario",
        "# exe = $Exe",
        "# hwnd = $hwnd",
        "# window_title = $($window.Title)",
        "# generated = $generated",
        "# wait_ms = $WaitMs",
        "# send_keys = $sendKeysHeader",
        "# click_menu = $clickMenuHeader",
        "# settle_ms = $SettleMs",
        "# nodes = $($script:nodes) (excluding_root = $nonRoot)",
        "# menu_items = $($script:menuItems)",
        "# depth_limit = $($script:maxDepth), node_limit = $($script:maxNodes)",
        "# traversal = TreeWalker.ControlViewWalker, 2 spaces per level",
        '# scope_note = dump 于合成输入（若有）之后；未使用合成输入时为初始树'
    )
    if ($script:truncated) { $header += '# truncated = true（达到节点上限）' }
    if ($script:depthLimited) { $header += '# depth_limited = true（达到深度上限）' }

    $allLines = [string[]]($header + $script:lines.ToArray())
    [IO.File]::WriteAllLines($OutFile, $allLines, [Text.UTF8Encoding]::new($false))

    if ($nonRoot -lt 2) {
        Write-Host ''
        Write-Host 'UIA 树为空：请确认使用 --features accessibility 构建（cargo build --release --bin veil --features accessibility）' -ForegroundColor Red
        Write-Host "除根节点外仅 $nonRoot 个节点；已把根元素写入：$OutFile" -ForegroundColor Yellow
        $exitCode = 1
    } else {
        Write-Host ''
        Write-Host "场景：$Scenario"
        Write-Host "窗口：HWND=$hwnd 标题=$($window.Title)"
        Write-Host "节点总数（含根）：$($script:nodes)，除根节点外：$nonRoot"
        Write-Host "MenuItem 数量：$($script:menuItems)"
        Write-Host "输出文件：$OutFile（UTF-8，$([IO.File]::ReadAllLines($OutFile).Count) 行）"
        if ($script:menuItems -eq 0) {
            Write-Host 'WARN：未发现 MenuItem，可能未启用 accessibility feature，或该窗口未暴露菜单语义。' -ForegroundColor Yellow
        }
        Write-Log 'PASS' "UIA 树已保存：$OutFile（nodes=$($script:nodes)，menu_items=$($script:menuItems)）"
        $exitCode = 0
    }
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    $exitCode = 1
} finally {
    if ($script:stderrPath -and (Test-Path -LiteralPath $script:stderrPath)) {
        $stderrInfo = Get-Item -LiteralPath $script:stderrPath
        if ($stderrInfo.Length -gt 0) {
            $tail = @(Get-Content -LiteralPath $script:stderrPath -Tail 20)
            Write-Log 'INFO' "stderr 输出（最后 $($tail.Count) 行）：$($tail -join ' | ')"
        }
    }
    Stop-VeilProcess -Process $script:proc
    # 失败时崩溃报告只落在临时 VEIL_HOME 里，清理前先复制到输出目录，保证失败原因可回溯。
    if ($script:tempRoot -and $script:outDir) {
        $logsDir = Join-Path $script:tempRoot 'home\logs'
        if (Test-Path -LiteralPath $logsDir) {
            foreach ($crash in @(Get-ChildItem -LiteralPath $logsDir -Filter 'veil-crash-*.txt' -ErrorAction SilentlyContinue)) {
                try {
                    $copyPath = Join-Path $script:outDir $crash.Name
                    Copy-Item -LiteralPath $crash.FullName -Destination $copyPath -Force
                    Write-Log 'WARN' "崩溃报告已保留：$copyPath"
                } catch {
                    Write-Log 'WARN' "复制崩溃报告失败：$($_.Exception.Message)"
                }
            }
        }
    }
    Remove-TempDir -Path $script:tempRoot
}

exit $exitCode
