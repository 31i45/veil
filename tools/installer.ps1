param(
  [string]$Version,
  [string]$BundleRoot,
  [string]$OutputDirectory,
  [string]$ISCCPath = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
)

$ErrorActionPreference = 'Stop'
# 原生命令统一由 $LASTEXITCODE 判定；不让 PowerShell 的 ErrorActionPreference 改写其失败语义。
$PSNativeCommandUseErrorActionPreference = $false
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

if (-not $Version) {
  $VersionMatch = Select-String -Path (Join-Path $RepoRoot 'Cargo.toml') -Pattern '^version = "([^"]+)"' | Select-Object -First 1
  if (-not $VersionMatch) { throw 'Could not read the workspace version from Cargo.toml' }
  $Version = $VersionMatch.Matches.Groups[1].Value
}
if (-not $BundleRoot) { $BundleRoot = Join-Path $RepoRoot 'release-bundle' }
if (-not $OutputDirectory) { $OutputDirectory = $BundleRoot }
$BundleRoot = [System.IO.Path]::GetFullPath($BundleRoot)
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
$SetupPath = Join-Path $OutputDirectory "veil-setup-$Version.exe"
$SetupChecksumPath = "$SetupPath.sha256"

function Write-Step($n, $desc) { Write-Host ''; Write-Host "== Step $n ($desc) ==" -ForegroundColor Cyan }
function Write-OK($msg)     { Write-Host "  OK  $msg" -ForegroundColor Green }
function Write-Warn($msg)   { Write-Host "  WARN $msg" -ForegroundColor Yellow }

function Remove-TargetArtifacts {
  foreach ($Path in @($SetupPath, $SetupChecksumPath)) {
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
      Remove-Item -LiteralPath $Path -Force -ErrorAction Stop
    }
  }
}

function Write-Fail($msg) {
  try { Remove-TargetArtifacts } catch { Write-Warn "could not clean setup artifacts: $($_.Exception.Message)" }
  Write-Host "  FAIL $msg" -ForegroundColor Red
  exit 1
}

trap {
  Write-Fail $_.Exception.Message
}

# 清理本次目标的陈旧产物；输入 bundle 内的 exe、sidecar 和发布记录绝不改写。
Remove-TargetArtifacts
if ($Version -notmatch '^\d+(?:\.\d+){0,3}$') {
  Write-Fail "Version must contain one to four numeric components for Inno Setup: '$Version'"
}

$BundleDir = Join-Path $BundleRoot "veil-v$Version"
$ExePath = Join-Path $BundleDir 'veil.exe'
$ChecksumPath = Join-Path $BundleDir 'veil.exe.sha256'
$RecordPath = Join-Path $BundleDir 'release-record.json'
$ScriptPath = Join-Path $RepoRoot 'installer\veil.iss'

Write-Host "Veil Installer v$Version" -ForegroundColor Magenta
Write-Host "Bundle: $BundleDir"
Write-Host "Output: $OutputDirectory"

Write-Step 1 'validate verified release bundle'
$RequiredBundleFiles = @('veil.exe', 'veil.exe.sha256', 'release-record.json')
foreach ($RelativePath in $RequiredBundleFiles) {
  if (-not (Test-Path -LiteralPath (Join-Path $BundleDir $RelativePath) -PathType Leaf)) {
    Write-Fail "required bundle file is missing: $RelativePath"
  }
}
$RequiredSourceFiles = @(
  'LICENSE',
  'THIRD-PARTY-LICENSES.md',
  'PRIVACY.md',
  'SECURITY.md',
  'SUPPORT.md',
  'icons\veil.ico'
)
foreach ($RelativePath in $RequiredSourceFiles) {
  if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot $RelativePath) -PathType Leaf)) {
    Write-Fail "required installer source file is missing: $RelativePath"
  }
}
# 合规文档随包取自仓库根目录；bundle 中已有同名副本时必须逐字节一致，
# 避免仓库文档与已验证发布产物漂移后被静默打进安装包。
foreach ($RelativePath in @('LICENSE', 'THIRD-PARTY-LICENSES.md', 'PRIVACY.md', 'SECURITY.md', 'SUPPORT.md')) {
  $BundleDoc = Join-Path $BundleDir $RelativePath
  if (-not (Test-Path -LiteralPath $BundleDoc -PathType Leaf)) { continue }
  $RepoHash = (Get-FileHash -LiteralPath (Join-Path $RepoRoot $RelativePath) -Algorithm SHA256).Hash
  $BundleHash = (Get-FileHash -LiteralPath $BundleDoc -Algorithm SHA256).Hash
  if ($RepoHash -cne $BundleHash) {
    Write-Fail "compliance document differs from verified bundle: $RelativePath"
  }
}

$ReleaseRecord = Get-Content -LiteralPath $RecordPath -Raw | ConvertFrom-Json
if ($ReleaseRecord.status -ne 'passed' -or $ReleaseRecord.release_ready -ne $true) {
  Write-Fail "release record is not ready: status=$($ReleaseRecord.status), release_ready=$($ReleaseRecord.release_ready)"
}
if ([string]$ReleaseRecord.version -cne $Version) {
  Write-Fail "release record version '$($ReleaseRecord.version)' does not match expected '$Version'"
}

$Exe = Get-Item -LiteralPath $ExePath -ErrorAction Stop
$ProductVersion = ([string]$Exe.VersionInfo.ProductVersion).Trim()
if ($ProductVersion -cne $Version) {
  Write-Fail "veil.exe ProductVersion '$ProductVersion' does not match expected '$Version'"
}

$SidecarMatch = [regex]::Match((Get-Content -LiteralPath $ChecksumPath -Raw).Trim(), '^([0-9A-Fa-f]{64})\s+veil\.exe$')
if (-not $SidecarMatch.Success) {
  Write-Fail 'veil.exe.sha256 must contain one SHA-256 hash followed by veil.exe'
}
$SidecarHash = $SidecarMatch.Groups[1].Value.ToLowerInvariant()
$ActualHash = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
$RecordHash = ([string]$ReleaseRecord.sha256).ToLowerInvariant()
if ($RecordHash -notmatch '^[0-9a-f]{64}$') {
  Write-Fail 'release record sha256 is missing or invalid'
}
if ($ActualHash -cne $SidecarHash -or $ActualHash -cne $RecordHash) {
  Write-Fail "SHA-256 mismatch: exe=$ActualHash, sidecar=$SidecarHash, record=$RecordHash"
}
Write-OK "release record passed and ready; version/ProductVersion=$Version"
Write-OK "verified veil.exe SHA-256: $ActualHash"

Write-Step 2 'compile Inno Setup installer'
if (-not (Test-Path -LiteralPath $ScriptPath -PathType Leaf)) {
  Write-Fail "installer script is missing: $ScriptPath"
}
if (-not (Test-Path -LiteralPath $ISCCPath -PathType Leaf)) {
  Write-Fail "ISCC was not found: $ISCCPath"
}
$ISCCPath = (Resolve-Path -LiteralPath $ISCCPath -ErrorAction Stop).Path
New-Item -ItemType Directory -Force -Path $OutputDirectory -ErrorAction Stop | Out-Null
$IsccArguments = @(
  '/Qp',
  "/DAppVersion=$Version",
  "/DBundleDir=$BundleDir",
  "/DRepoRoot=$RepoRoot",
  "/DOutputDirectory=$OutputDirectory",
  $ScriptPath
)
$IsccOutput = @(& $ISCCPath @IsccArguments 2>&1)
$IsccExit = $LASTEXITCODE
$IsccOutput | Out-Host
if ($IsccExit -ne 0) {
  Write-Fail "ISCC failed with exit $IsccExit"
}
if (-not (Test-Path -LiteralPath $SetupPath -PathType Leaf)) {
  Write-Fail "ISCC succeeded but did not create $SetupPath"
}

$Setup = Get-Item -LiteralPath $SetupPath -ErrorAction Stop
$SetupProductVersion = ([string]$Setup.VersionInfo.ProductVersion).Trim()
if ($SetupProductVersion -cne $Version) {
  Write-Fail "setup ProductVersion '$SetupProductVersion' does not match expected '$Version'"
}
Write-OK "setup compiled with ProductVersion $SetupProductVersion ($($Setup.Length) bytes)"

Write-Step 3 'generate and validate setup SHA-256'
$SetupHash = (Get-FileHash -LiteralPath $SetupPath -Algorithm SHA256).Hash.ToLowerInvariant()
$ChecksumLine = "$SetupHash  veil-setup-$Version.exe"
Set-Content -LiteralPath $SetupChecksumPath -Value $ChecksumLine -Encoding ascii
$Expected = ((Get-Content -LiteralPath $SetupChecksumPath -Raw).Trim() -split '\s+')[0]
$Actual = (Get-FileHash -LiteralPath $SetupPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Expected -cne $SetupHash -or $Actual -cne $Expected) {
  Write-Fail 'setup SHA-256 sidecar validation failed'
}
Write-OK "veil-setup-$Version.exe.sha256 = $ChecksumLine"
Write-Warn 'Setup is unsigned; production Authenticode signing: NOT RUN'
Write-Host ''
Write-OK "Installer ready: $SetupPath"
exit 0
