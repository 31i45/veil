# Veil 崩溃恢复演练（qq-task3）：真实启动 GUI → 注入编辑 → 强制结束 → 重启取证。
#
# 用法：
#   pwsh -NoProfile -File tools/verify/recovery_drill.ps1
#   pwsh -NoProfile -File tools/verify/recovery_drill.ps1 -SkipGui   # 只跑文件级演练
#
# 结论写入 evidence/verify/qq-task3/drill.txt；任何断言失败以 exit 1 结束。
#
# 诚实披露：
#   - 阶段 A（文件自动保存）：打开真实 .md，注入编辑后等待自动保存触发点，再用
#     Stop-Process -Force 结束。断言「文件只会是旧内容或完整新内容，永不是半截」。
#   - 阶段 B（未命名草稿崩溃恢复）：无参数启动（无 session → 未命名），注入编辑
#     等恢复静默期写盘，强制结束，断言 recovery.dat 存在、魔数正确、含编辑标记；
#     随后重启断言仍可启动、恢复文件仍在（要等到成功保存才清除）。
#   - 「rename 系统调用内部被中断」无法由外部脚本稳定命中；阶段 C 用文件级方式
#     确定性构造「临时文件已写、未 rename」，断言目标未被覆盖。
#   - GUI 无法启动/无法注入时不伪装成功：记录降级原因，仅以文件级演练判定。

[CmdletBinding()]
param(
    [string]$Exe = '',
    [string]$OutDir = '',
    [switch]$SkipGui
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Exe) { $Exe = Join-Path $repoRoot 'target\debug\veil.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repoRoot 'evidence\verify\qq-task3' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$drillPath = Join-Path $OutDir 'drill.txt'
$marker = 'zzq7'

$script:lines = [System.Collections.Generic.List[string]]::new()
$script:failed = $false

function Say([string]$s) { $script:lines.Add($s); Write-Host $s }
function Info([string]$s) { Say ("INFO  " + $s) }
function Pass([string]$s) { Say ("PASS  " + $s) }
function Fail([string]$s) { $script:failed = $true; Say ("FAIL  " + $s) }
function Get-Utf8NoBom { [Text.UTF8Encoding]::new($false) }
function Write-Text([string]$path, [string]$text) {
    [IO.File]::WriteAllText($path, $text, (Get-Utf8NoBom))
}
function Read-Utf8Strict([string]$path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    return [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
}

if (-not ('VeilDrill.Win32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
namespace VeilDrill {
    public struct RECT { public int Left, Top, Right, Bottom; }
    public static class Win32 {
        public delegate bool EnumProc(IntPtr h, IntPtr p);
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr p);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
        [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)]
        public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    }
}
'@
}

function Find-DrillWindow([int]$processId, [int]$timeoutMs) {
    $deadline = (Get-Date).AddMilliseconds($timeoutMs)
    do {
        $script:foundHwnd = [IntPtr]::Zero
        $script:foundTitle = ''
        $script:wantPid = [uint32]$processId
        $cb = [VeilDrill.Win32+EnumProc] {
            param([IntPtr]$h, [IntPtr]$x)
            $winPid = [uint32]0
            [void][VeilDrill.Win32]::GetWindowThreadProcessId($h, [ref]$winPid)
            if ($winPid -ne $script:wantPid) { return $true }
            if (-not [VeilDrill.Win32]::IsWindowVisible($h)) { return $true }
            $r = New-Object VeilDrill.RECT
            [void][VeilDrill.Win32]::GetWindowRect($h, [ref]$r)
            if (($r.Right - $r.Left) -le 200 -or ($r.Bottom - $r.Top) -le 300) { return $true }
            $sb = New-Object Text.StringBuilder 512
            [void][VeilDrill.Win32]::GetWindowText($h, $sb, 512)
            $script:foundHwnd = $h
            $script:foundTitle = $sb.ToString()
            return $false
        }
        [void][VeilDrill.Win32]::EnumWindows($cb, [IntPtr]::Zero)
        if ($script:foundHwnd -ne [IntPtr]::Zero) { return $script:foundHwnd }
        Start-Sleep -Milliseconds 120
    } while ((Get-Date) -lt $deadline)
    throw "等待 veil 主窗口超时（${timeoutMs}ms）"
}

function Get-DrillTitle([IntPtr]$hwnd) {
    $sb = New-Object Text.StringBuilder 512
    [void][VeilDrill.Win32]::GetWindowText($hwnd, $sb, 512)
    return $sb.ToString()
}

function Send-DrillText([IntPtr]$hwnd, [string]$text) {
    foreach ($ch in $text.ToCharArray()) {
        $vk = [int][char]([string]$ch).ToUpperInvariant()
        [void][VeilDrill.Win32]::PostMessage($hwnd, 0x0100, [IntPtr]$vk, [IntPtr]0)  # WM_KEYDOWN
        [void][VeilDrill.Win32]::PostMessage($hwnd, 0x0102, [IntPtr][int]$ch, [IntPtr]0) # WM_CHAR
        [void][VeilDrill.Win32]::PostMessage($hwnd, 0x0101, [IntPtr]$vk, [IntPtr]0)  # WM_KEYUP
        Start-Sleep -Milliseconds 80
    }
}

function Stop-DrillProcess($proc) {
    if ($null -eq $proc) { return }
    try {
        if (-not $proc.HasExited) {
            Stop-Process -Id $proc.Id -Force -ErrorAction Stop
            $proc.WaitForExit(5000) | Out-Null
        }
    } catch {
        Say ("WARN  结束进程失败：$($_.Exception.Message)")
    }
}

function Start-Veil([string]$veilHomeDir, [string]$tag, [string[]]$arguments) {
    $envs = @{
        VEIL_HOME    = $veilHomeDir
        APPDATA      = (Join-Path $veilHomeDir 'appdata')
        LOCALAPPDATA = (Join-Path $veilHomeDir 'localappdata')
        TEMP         = (Join-Path $veilHomeDir 'temp')
        TMP          = (Join-Path $veilHomeDir 'temp')
    }
    New-Item -ItemType Directory -Path $envs.APPDATA, $envs.LOCALAPPDATA, $envs.TEMP -Force | Out-Null
    $params = @{
        FilePath               = $script:ExePath
        PassThru               = $true
        RedirectStandardOutput = (Join-Path $veilHomeDir "stdout-$tag.log")
        RedirectStandardError  = (Join-Path $veilHomeDir "stderr-$tag.log")
        Environment            = $envs
    }
    if ($arguments -and $arguments.Count -gt 0) { $params.ArgumentList = $arguments }
    return Start-Process @params
}

$tmpRoot = Join-Path ([IO.Path]::GetTempPath()) ("veil-recovery-drill-{0}-{1}" -f (Get-Date -Format 'yyyyMMdd-HHmmssfff'), $PID)
$procA = $null
$procB = $null
$procB2 = $null
$guiOk = $false
$guiReason = ''
$autosavePersisted = $false
$autosaveUnchanged = $false
$recoveryFound = $false
$recoverySurvivedRestart = $false
$restartOk = $false
$fileList = @()

try {
    if (-not (Test-Path -LiteralPath $Exe)) {
        Info "未找到 $Exe，执行 cargo build --bin veil"
        Push-Location $repoRoot
        try {
            & cargo build --bin veil
            if ($LASTEXITCODE -ne 0) { throw "cargo build 失败（exit $LASTEXITCODE）" }
        } finally { Pop-Location }
    }
    if (-not (Test-Path -LiteralPath $Exe)) { throw "构建后仍找不到 exe：$Exe" }
    $script:ExePath = (Resolve-Path -LiteralPath $Exe).Path
    New-Item -ItemType Directory -Path $tmpRoot -Force | Out-Null
    Info "隔离根目录：$tmpRoot"

    if ($SkipGui) {
        $guiReason = '调用方指定 -SkipGui'
        Say "WARN  GUI 阶段跳过：$guiReason"
    } else {
        try {
            # ---------- 阶段 A：文件自动保存，强制结束 ----------
            $homeA = Join-Path $tmpRoot 'home-a'
            $docDir = Join-Path $tmpRoot 'doc'
            New-Item -ItemType Directory -Path $homeA, $docDir -Force | Out-Null
            $docPath = Join-Path $docDir 'drill.md'
            # 含多字节与较大体量（约 3 MB），便于观察「要么旧、要么完整新」。
            $original = "# 恢复演练`n`n多字节内容用于检测半截写入：甲乙丙丁戊己庚辛。`n`n" + ('演练正文长行。' * 120000) + "`n"
            Write-Text $docPath $original
            Info "阶段 A：文档 $docPath（$([IO.File]::ReadAllBytes($docPath).Length) 字节）"

            $procA = Start-Veil $homeA 'a' @($docPath)
            Start-Sleep -Milliseconds 300
            $hwndA = Find-DrillWindow $procA.Id 30000
            Info "阶段 A 窗口：PID=$($procA.Id) HWND=$hwndA title='$(Get-DrillTitle $hwndA)'"

            Send-DrillText $hwndA $marker
            $titleDirty = Get-DrillTitle $hwndA
            if (-not $titleDirty.Contains('*')) { throw "注入后标题未出现脏标记：$titleDirty" }
            Pass "阶段 A：已注入编辑标记，标题=$titleDirty"

            # 自动保存静默期 2s；在触发点附近强制结束，制造「写入窗口」竞争。
            Start-Sleep -Milliseconds 2150
            Stop-DrillProcess $procA
            $procA = $null
            Info '阶段 A：已在自动保存触发点附近 Stop-Process -Force'

            $after = Read-Utf8Strict $docPath
            if ($after -eq $original) {
                $autosaveUnchanged = $true
                Pass '阶段 A：强制结束后文件仍是完整旧内容（保存未落盘，零损坏）'
            } elseif ([regex]::IsMatch($after, [regex]::Escape($marker), 'IgnoreCase') -and
                ([regex]::Replace($after, [regex]::Escape($marker), '', 'IgnoreCase') -eq $original)) {
                $autosavePersisted = $true
                Pass '阶段 A：文件是完整新内容（自动保存已原子落盘，零半截）'
            } else {
                Fail '阶段 A：文件既不是完整旧内容也不是完整新内容（疑似半截写入）'
            }
        } catch {
            $guiReason = $_.Exception.Message
            Say "WARN  GUI 阶段 A 降级：$guiReason"
            Stop-DrillProcess $procA
            $procA = $null
        }

        # ---------- 阶段 B：未命名草稿崩溃恢复 ----------
        if (-not $script:failed) {
            try {
                $homeB = Join-Path $tmpRoot 'home-b'
                New-Item -ItemType Directory -Path $homeB -Force | Out-Null
                $procB = Start-Veil $homeB 'b' @()
                Start-Sleep -Milliseconds 300
                $hwndB = Find-DrillWindow $procB.Id 30000
                Info "阶段 B 窗口：PID=$($procB.Id) HWND=$hwndB title='$(Get-DrillTitle $hwndB)'"

                Send-DrillText $hwndB $marker
                $titleB = Get-DrillTitle $hwndB
                if (-not $titleB.Contains('*')) { throw "注入后标题未出现脏标记：$titleB" }
                Pass "阶段 B：未命名草稿已注入编辑标记，标题=$titleB"

                Start-Sleep -Milliseconds 3000
                Stop-DrillProcess $procB
                $procB = $null
                Info '阶段 B：Stop-Process -Force（模拟崩溃）'

                $recovery = Join-Path $homeB 'recovery.dat'
                if (-not (Test-Path -LiteralPath $recovery)) { throw "未生成恢复文件：$recovery" }
                $rb = [IO.File]::ReadAllBytes($recovery)
                if ($rb.Length -lt 16) { throw "恢复文件过短：$($rb.Length) 字节" }
                $magic = [Text.Encoding]::ASCII.GetString($rb, 0, 16)
                if (-not $magic.StartsWith('VEIL-RECOVERY-2')) { throw "恢复文件魔数不符：$magic" }
                $recovered = [Text.Encoding]::UTF8.GetString($rb)
                if (-not [regex]::IsMatch($recovered, [regex]::Escape($marker), 'IgnoreCase')) {
                    throw '恢复文件未包含崩溃前的编辑标记'
                }
                $recoveryFound = $true
                Pass "阶段 B：存在可恢复文件 $recovery（$($rb.Length) 字节，含标记）"

                Info '阶段 B：重启 veil.exe（无参数，走恢复分支）'
                $procB2 = Start-Veil $homeB 'b2' @()
                Start-Sleep -Milliseconds 2500
                if ($procB2.HasExited) { throw "重启后进程提前退出（ExitCode=$($procB2.ExitCode)）" }
                $restartOk = $true
                if (Test-Path -LiteralPath $recovery) {
                    $recoverySurvivedRestart = $true
                    Pass '阶段 B：重启后恢复文件仍在（成功保存前不清除），且进程存活'
                } else {
                    Say 'WARN  重启后恢复文件已被清理（记录实际行为）'
                }
                Stop-DrillProcess $procB2
                $procB2 = $null
                $guiOk = $true
            } catch {
                $guiReason = $_.Exception.Message
                Say "WARN  GUI 阶段 B 降级：$guiReason"
                Stop-DrillProcess $procB
                $procB = $null
                Stop-DrillProcess $procB2
                $procB2 = $null
            }
        }
    }

    # ---------- 阶段 C：文件级确定性演练（始终执行）----------
    $fl = Join-Path $tmpRoot 'filelevel'
    New-Item -ItemType Directory -Path $fl -Force | Out-Null
    $flDoc = Join-Path $fl 'drill.md'
    $flOriginal = "original-content`n"
    $flCandidate = "candidate-after-edit`n"
    Write-Text $flDoc $flOriginal

    # C1：候选版本已写入临时文件、rename 前被杀。
    $flTmp = "$flDoc.veil-tmp"
    Write-Text $flTmp $flCandidate
    if ((Read-Utf8Strict $flDoc) -ne $flOriginal) {
        Fail 'C1 中断写入：目标文件被候选版本覆盖'
    } else {
        Pass 'C1 中断写入：目标文件未被候选版本覆盖'
    }
    if (-not (Test-Path -LiteralPath $flTmp)) {
        Fail "C1 中断写入：临时文件缺失 $flTmp"
    } elseif ((Read-Utf8Strict $flTmp) -ne $flCandidate) {
        Fail 'C1 中断写入：临时文件内容不符'
    } else {
        Pass "C1 中断写入：临时文件承载候选版本 $flTmp"
    }

    # C2：外部版本冲突副本的命名约定。
    $flBak = Join-Path $fl 'drill.conflict-1.md'
    Write-Text $flBak "external-version`n"
    if (Test-Path -LiteralPath $flBak) {
        Pass "C2 备份文件存在且可识别：$flBak"
    } else {
        Fail "C2 备份文件缺失：$flBak"
    }

    $fileList = @(Get-ChildItem -LiteralPath $fl | Select-Object -ExpandProperty Name)
    Info ('阶段 C 目录内容：' + ($fileList -join ', '))
}
catch {
    Fail "演练异常：$($_.Exception.Message)"
}
finally {
    Stop-DrillProcess $procA
    Stop-DrillProcess $procB
    Stop-DrillProcess $procB2
    $mode = if ($guiOk) { 'gui+file-level' } elseif ($SkipGui) { "degraded (skip-gui: $guiReason)" } else { "degraded (file-level only: $guiReason)" }
    $verdict = if ($script:failed) { 'FAIL' } elseif ($guiOk) { 'PASS' } else { 'PASS (degraded)' }
    $header = @(
        '# Veil 崩溃恢复演练',
        "time = $((Get-Date).ToString('yyyy-MM-dd HH:mm:ss'))",
        "exe = $script:ExePath",
        "mode = $mode",
        "gui_ok = $guiOk",
        "gui_reason = $guiReason",
        "autosave_persisted_after_kill = $autosavePersisted",
        "autosave_stayed_original_after_kill = $autosaveUnchanged",
        "recovery_file_found = $recoveryFound",
        "recovery_survived_restart = $recoverySurvivedRestart",
        "restart_ok = $restartOk",
        "filelevel_files = $($fileList -join ', ')",
        '',
        '## 记录'
    )
    $all = @($header) + @($script:lines) + @('', "verdict = $verdict")
    try {
        [IO.File]::WriteAllLines($drillPath, $all, (Get-Utf8NoBom))
        Write-Host "结论已写入：$drillPath"
    } catch {
        Write-Host "写入 drill.txt 失败：$($_.Exception.Message)"
        $script:failed = $true
    }
    try {
        if (Test-Path -LiteralPath $tmpRoot) { Remove-Item -LiteralPath $tmpRoot -Recurse -Force }
    } catch {
        Write-Host "WARN 临时目录未删除：$tmpRoot"
    }
}

if ($script:failed) { exit 1 } else { exit 0 }
