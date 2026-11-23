# Builds veil-setup-<version>.exe from a verified release bundle.
# Authenticode signing is opt-in: pass -CertificatePath and -CertificatePassword to sign and
# timestamp the final setup; otherwise the setup is built unsigned and reported as such.
# A self-signed certificate is trusted only by machines that explicitly trust it and is not
# a production trust anchor.
param(
  [string]$Version,
  [string]$BundleRoot,
  [string]$OutputDirectory,
  [string]$ISCCPath = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
  [string]$CertificatePath,
  [string]$CertificatePassword,
  [string]$TimestampUrl = 'http://timestamp.digicert.com'
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

Write-Step 3 'optional Authenticode signing of setup'
$SetupIsSigned = $false
if ([string]::IsNullOrWhiteSpace($CertificatePath)) {
  if ($PSBoundParameters.ContainsKey('CertificatePassword')) {
    Write-Fail 'CertificatePassword requires CertificatePath'
  }
  Write-Warn 'Setup is unsigned; production Authenticode signing: NOT RUN'
} else {
  if ([string]::IsNullOrWhiteSpace($CertificatePassword)) {
    Write-Fail 'CertificatePath requires a non-empty CertificatePassword'
  }
  if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
    Write-Fail 'TimestampUrl must not be empty when signing'
  }
  if (-not (Test-Path -LiteralPath $CertificatePath -PathType Leaf)) {
    Write-Fail "signing certificate PFX was not found: $CertificatePath"
  }
  $ResolvedCertificate = (Resolve-Path -LiteralPath $CertificatePath -ErrorAction Stop).Path

  $SignTool = Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1
  $SignToolExe = if ($SignTool) { $SignTool.Source } else { $null }
  if (-not $SignToolExe) {
    $SdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $SignToolCandidate = Get-ChildItem -LiteralPath $SdkBin -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\' } |
      Sort-Object FullName -Descending |
      Select-Object -First 1
    if ($SignToolCandidate) { $SignToolExe = $SignToolCandidate.FullName }
  }
  if (-not $SignToolExe) { Write-Fail 'Authenticode signing requested, but signtool.exe was not found' }
  Write-OK "signtool: $SignToolExe"

  # The setup must not wrap an unsigned payload; the bundle exe was already hash-verified above.
  $PayloadSignature = Get-AuthenticodeSignature -LiteralPath $ExePath
  if ($PayloadSignature.Status -ne 'Valid') {
    Write-Fail "bundle payload veil.exe is not validly Authenticode-signed (status=$($PayloadSignature.Status)); refusing to wrap an unsigned payload in a signed setup"
  }
  $PayloadSignerSubject = if ($PayloadSignature.SignerCertificate) { $PayloadSignature.SignerCertificate.Subject } else { '<unknown>' }
  Write-OK "payload veil.exe Authenticode status: Valid (signer: $PayloadSignerSubject)"

  # Password is passed as an argument only and is never echoed or written to disk.
  $SignArguments = @(
    'sign',
    '/fd', 'SHA256',
    '/f', $ResolvedCertificate,
    '/p', $CertificatePassword,
    '/tr', $TimestampUrl,
    '/td', 'SHA256',
    '/v',
    $SetupPath
  )
  & $SignToolExe @SignArguments | Out-Host
  $SignExit = $LASTEXITCODE
  if ($SignExit -ne 0) {
    Write-Fail "signtool sign failed with exit $SignExit"
  }

  $VerifyOutput = @(& $SignToolExe verify /pa /all /v $SetupPath 2>&1)
  $VerifyExit = $LASTEXITCODE
  $VerifyOutput | Out-Host
  if ($VerifyExit -ne 0) {
    Write-Fail "Authenticode verification failed with exit ${VerifyExit}: $($VerifyOutput -join ' ')"
  }
  $VerifyText = $VerifyOutput -join "`n"
  if ($VerifyText -notmatch 'The signature is timestamped:' -or $VerifyText -notmatch 'Timestamp Verified by:') {
    Write-Fail 'RFC3161 timestamp evidence is missing or was not verified by signtool'
  }

  $SetupSignature = Get-AuthenticodeSignature -LiteralPath $SetupPath
  if ($SetupSignature.Status -ne 'Valid') {
    Write-Fail "setup Authenticode status is $($SetupSignature.Status) after signing; expected Valid"
  }
  $SetupSignerSubject = if ($SetupSignature.SignerCertificate) { $SetupSignature.SignerCertificate.Subject } else { '<unknown>' }
  $SetupSignerThumbprint = if ($SetupSignature.SignerCertificate) { $SetupSignature.SignerCertificate.Thumbprint } else { '<unknown>' }
  Write-OK "setup Authenticode status: Valid"
  Write-OK "signer subject: $SetupSignerSubject"
  Write-OK "signer thumbprint: $SetupSignerThumbprint"
  if ($SetupSignature.TimeStamperCertificate) {
    Write-OK "RFC3161 timestamp signer: $($SetupSignature.TimeStamperCertificate.Subject)"
  } else {
    Write-Warn 'Get-AuthenticodeSignature exposed no timestamper certificate; signtool verify did report a verified timestamp'
  }
  Write-Warn 'Signed setup is trusted only when the certificate chains to a trusted root; a self-signed certificate is not a production trust anchor'
  $SetupIsSigned = $true
}

Write-Step 4 'generate and validate setup SHA-256'
$SetupHash = (Get-FileHash -LiteralPath $SetupPath -Algorithm SHA256).Hash.ToLowerInvariant()
$ChecksumLine = "$SetupHash  veil-setup-$Version.exe"
Set-Content -LiteralPath $SetupChecksumPath -Value $ChecksumLine -Encoding ascii
$Expected = ((Get-Content -LiteralPath $SetupChecksumPath -Raw).Trim() -split '\s+')[0]
$Actual = (Get-FileHash -LiteralPath $SetupPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Expected -cne $SetupHash -or $Actual -cne $Expected) {
  Write-Fail 'setup SHA-256 sidecar validation failed'
}
Write-OK "veil-setup-$Version.exe.sha256 = $ChecksumLine"
Write-Host ''
if ($SetupIsSigned) {
  Write-OK "Installer ready (Authenticode signed + timestamped): $SetupPath"
} else {
  Write-OK "Installer ready (unsigned): $SetupPath"
}
exit 0
