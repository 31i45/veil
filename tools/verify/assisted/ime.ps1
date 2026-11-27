# Veil 真机中文 IME assisted 取证（Task 5）
#
# 用户只做一件事：用真实输入法完成六步操作（打字/选词/Esc/失焦回焦）。
# 脚本负责：启动 release veil.exe、逐步提示、回车后自动截图、生成 ime-result.md 判定表。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/assisted/ime.ps1              # 交互执行六步走查
#   pwsh -NoProfile -File tools/verify/assisted/ime.ps1 -Preflight    # 不执行，仅登记 NOT RUN
#
# 六步（每个输入法各走一遍：微软拼音 / 微软五笔）：
#   1) 段落中部输入 nihao，preedit 下划线跟随光标、不越出版心
#   2) 空格上屏「你好」，无重复字符、无光标跳动
#   3) 输入 zhongguo 后按数字键选第 2 候选，上屏文字正确
#   4) 在行首、行尾、表格单元格内各重复一次步骤 1–3
#   5) 输入中按 Esc 取消组合，文档字节不变
#   6) 输入中窗口失焦再回焦，候选窗位置与 preedit 状态一致
#
# 退出码：0 = 六步均完成抓图；2 = 未执行（NOT RUN）；1 = 启动或抓图失败。

[CmdletBinding()]
param(
    [switch]$Preflight,
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
$resultPath = Join-Path $OutDir 'ime-result.md'

$steps = @(
    @{ N = 1; Title = '段落中部输入 nihao'; Detail = 'preedit 下划线跟随光标、不越出版心' },
    @{ N = 2; Title = '空格上屏「你好」'; Detail = '无重复字符、无光标跳动' },
    @{ N = 3; Title = '输入 zhongguo 选第 2 候选'; Detail = '按数字键 2，上屏文字正确' },
    @{ N = 4; Title = '行首/行尾/表格单元格重复 1–3'; Detail = '三处各做一次输入、上屏、选词' },
    @{ N = 5; Title = '输入中按 Esc 取消组合'; Detail = '文档字节不变（可与编辑前字数对比）' },
    @{ N = 6; Title = '窗口失焦再回焦'; Detail = '候选窗位置与 preedit 状态一致' }
)

function Write-ResultTable {
    param([array]$Rows, [string]$Verdict, [string]$Note)
    $lines = foreach ($r in $Rows) {
        $shot = if ($r.Screenshot) { "``$($r.Screenshot)``" } else { '—' }
        "| $($r.N) | $($r.Title) | $($r.Observation) | $shot | $($r.Status) |"
    }
    $md = @"
# 真机 IME 六步走查结果（$dateStamp）

> 本文件由 ``tools/verify/assisted/ime.ps1`` 自动生成。
> 纪律：未真实执行时记 **NOT RUN**，不写「预期通过」。

- 时间：$((Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz'))
- 机器：$env:COMPUTERNAME
- 语料：``$Doc``
- 结论：$Verdict
- 说明：$Note

## 六步结果

| 步骤 | 用户操作 | 用户观察 | 截图 | 判定 |
|---|---|---|---|---|
$($lines -join "`n")

## 用户只需执行的那一条命令

``````powershell
pwsh -NoProfile -File tools/verify/assisted/ime.ps1
``````

执行前把系统输入法切到「微软拼音」，并按提示在行首/行尾/表格单元格重复；脚本会在每个回车后自动截图。
"@
    [IO.File]::WriteAllText($resultPath, $md, [Text.UTF8Encoding]::new($false))
    Write-Host "结果已写入：$resultPath"
}

function New-SkippedRows {
    return @($steps | ForEach-Object {
            [pscustomobject]@{ N = $_.N; Title = $_.Title; Observation = '未执行'; Screenshot = $null; Status = 'NOT RUN' }
        })
}

if (-not $Doc) {
    $matrix = Join-Path $repoRoot 'docs\qa\syntax-matrix.md'
    $Doc = if (Test-Path -LiteralPath $matrix) { $matrix } else { Join-Path $repoRoot 'README.zh-CN.md' }
}

if ($Preflight) {
    Write-ResultTable -Rows (New-SkippedRows) -Verdict '**NOT RUN**' -Note '调用方指定 -Preflight：未启动应用、未进行真实输入。'
    Write-Host 'IME 六步走查本轮未执行（-Preflight）。用户侧一条命令见 ime-result.md。'
    exit 2
}

if ([Console]::IsInputRedirected) {
    # 自动化环境没有可交互 stdin：拒绝启动应用并“自动回车”，否则会生成用户没有真正操作的假截图。
    Write-ResultTable -Rows (New-SkippedRows) -Verdict '**NOT RUN**' -Note '检测到标准输入被重定向：本脚本需要真人逐步操作与回车，拒绝在非交互环境生成无操作截图。'
    Write-Host '当前标准输入被重定向，无法进行真人六步走查。请在有控制台的终端里运行：' -ForegroundColor Yellow
    Write-Host '  pwsh -NoProfile -File tools/verify/assisted/ime.ps1'
    exit 2
}

if (-not (Test-Path -LiteralPath $Exe)) {
    Write-ResultTable -Rows (New-SkippedRows) -Verdict '**NOT RUN**' -Note "缺少 $Exe，请先 cargo build --release -p veil"
    Write-Host "找不到 release 可执行文件：$Exe（先构建再重跑）" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path -LiteralPath $Doc)) { throw "找不到测试语料：$Doc" }

Add-Type -AssemblyName System.Drawing
if (-not ('VeilAssistedIme.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace VeilAssistedIme {
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

[VeilAssistedIme.Native]::SetProcessDPIAware() | Out-Null

function Get-ProcessWindows {
    param([int]$ProcessId)
    $script:enumPid = [uint32]$ProcessId
    $script:enumWindows = [System.Collections.Generic.List[object]]::new()
    $collect = {
        param([IntPtr]$h, [IntPtr]$p)
        $wp = [uint32]0
        [VeilAssistedIme.Native]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
        if ($wp -ne $script:enumPid) { return $true }
        if (-not [VeilAssistedIme.Native]::IsWindowVisible($h)) { return $true }
        if ([VeilAssistedIme.Native]::IsIconic($h)) { return $true }
        $r = New-Object VeilAssistedIme.RECT
        [VeilAssistedIme.Native]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.Right - $r.Left) -lt 200 -or ($r.Bottom - $r.Top) -lt 300) { return $true }
        $title = New-Object System.Text.StringBuilder 512
        [VeilAssistedIme.Native]::GetWindowText($h, $title, $title.Capacity) | Out-Null
        $script:enumWindows.Add([pscustomobject]@{ Hwnd = $h; Title = $title.ToString() })
        return $true
    }
    [VeilAssistedIme.Native]::EnumWindows($collect, [IntPtr]::Zero) | Out-Null
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
    $wr = New-Object VeilAssistedIme.RECT
    [VeilAssistedIme.Native]::GetWindowRect($Hwnd, [ref]$wr) | Out-Null
    $w = $wr.Right - $wr.Left
    $h = $wr.Bottom - $wr.Top
    if ($w -le 0 -or $h -le 0) { throw "窗口尺寸无效 ${w}x${h}" }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $printed = $false
    for ($attempt = 0; $attempt -lt 3 -and -not $printed; $attempt++) {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        try {
            $printed = [VeilAssistedIme.Native]::PrintWindow($Hwnd, $hdc, [VeilAssistedIme.K]::PW_RENDERFULLCONTENT)
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
        $visible = New-Object VeilAssistedIme.RECT
        $dwmOk = [VeilAssistedIme.Native]::DwmGetWindowAttribute($Hwnd, 9, [ref]$visible, 16) -eq 0
        if (-not $dwmOk -or ($visible.Right - $visible.Left) -le 0 -or ($visible.Bottom - $visible.Top) -le 0) {
            $visible = $wr
        }
        $vw = $visible.Right - $visible.Left
        $vh = $visible.Bottom - $visible.Top
        $flags = ([VeilAssistedIme.K]::SWP_NOMOVE -bor [VeilAssistedIme.K]::SWP_NOSIZE -bor [VeilAssistedIme.K]::SWP_NOACTIVATE)
        $screen = New-Object System.Drawing.Bitmap($vw, $vh)
        [VeilAssistedIme.Native]::SetWindowPos($Hwnd, [VeilAssistedIme.K]::HWND_TOPMOST, 0, 0, 0, 0, $flags) | Out-Null
        try {
            Start-Sleep -Milliseconds 300
            $g2 = [System.Drawing.Graphics]::FromImage($screen)
            try {
                $g2.CopyFromScreen($visible.Left, $visible.Top, 0, 0, (New-Object System.Drawing.Size($vw, $vh)))
            } finally { $g2.Dispose() }
        } finally {
            [VeilAssistedIme.Native]::SetWindowPos($Hwnd, [VeilAssistedIme.K]::HWND_NOTOPMOST, 0, 0, 0, 0, $flags) | Out-Null
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
    [pscustomobject]@{ Path = $Path; Bytes = (Get-Item -LiteralPath $Path).Length; Width = $width; Height = $height; Method = $method }
}

$proc = $null
$tempRoot = ''
try {
    $stampIso = Get-Date -Format 'yyyyMMdd-HHmmss'
    $tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-ime-$stampIso-$PID")
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
    Start-Sleep -Milliseconds 1500
    [void][VeilAssistedIme.Native]::ShowWindow($hwnd, [VeilAssistedIme.K]::SW_RESTORE)
    for ($i = 0; $i -lt 5; $i++) {
        [void][VeilAssistedIme.Native]::SetForegroundWindow($hwnd)
        Start-Sleep -Milliseconds 150
        if ([VeilAssistedIme.Native]::GetForegroundWindow() -eq $hwnd) { break }
    }
    Write-Host '已启动 veil.exe。请把系统输入法切到「微软拼音」；每一步完成后回车，脚本自动截图。'

    $rows = [System.Collections.Generic.List[object]]::new()
    foreach ($step in $steps) {
        Write-Host ''
        Write-Host ("=== 步骤 {0}/6：{1} ===" -f $step.N, $step.Title) -ForegroundColor Cyan
        Write-Host ("   判定要点：{0}" -f $step.Detail)
        $obs = Read-Host '   完成本步后直接回车（也可先输入观察，再回车）'
        Start-Sleep -Milliseconds 400
        $png = Join-Path $OutDir ("ime-step-{0}.png" -f $step.N)
        try {
            $shot = Save-WindowCapture -Hwnd $hwnd -Path $png
            $rows.Add([pscustomobject]@{
                    N = $step.N; Title = $step.Title
                    Observation = if ([string]::IsNullOrWhiteSpace($obs)) { '（未填写观察）' } else { $obs }
                    Screenshot = $shot.Path; Status = 'CAPTURED'
                })
            Write-Host ("   已截图：{0}（{1}x{2}）" -f $shot.Path, $shot.Width, $shot.Height)
        } catch {
            $rows.Add([pscustomobject]@{
                    N = $step.N; Title = $step.Title
                    Observation = if ([string]::IsNullOrWhiteSpace($obs)) { '（未填写观察）' } else { $obs }
                    Screenshot = $null; Status = "FAIL（$($_.Exception.Message)）"
                })
            Write-Host "   截图失败：$($_.Exception.Message)" -ForegroundColor Red
        }
    }

    Write-Host ''
    $answer = Read-Host '六步是否均无丢字/重字/错位？输入 y = PASS，其它 = 记 FAIL'
    $verdict = if ($answer -match '^(y|yes|是)$') { '**PASS**（六步无丢字/重字/错位）' } else { '**FAIL**（用户判定未通过，见观察列）' }
    Write-ResultTable -Rows $rows -Verdict $verdict -Note '用户完成六步物理操作，脚本逐步自动截图。'
    if ($verdict -like '**FAIL**') { exit 1 } else { exit 0 }
} catch {
    Write-Host "失败：$($_.Exception.Message)" -ForegroundColor Red
    Write-ResultTable -Rows (New-SkippedRows) -Verdict '**NOT RUN**' -Note "执行失败：$($_.Exception.Message)"
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
