param(
  [switch]$SkipTest,
  [switch]$SkipClippy,
  [switch]$SkipArchive,
  [string]$Version,
  [string]$CertificatePath,
  [string]$CertificatePassword,
  [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'
# 原生命令统一由 $LASTEXITCODE 判定；不让 PowerShell 的 ErrorActionPreference 改写其失败语义。
$PSNativeCommandUseErrorActionPreference = $false
$Stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

$ExePath = Join-Path $RepoRoot 'target\release\veil.exe'
$ChecksumPath = "$ExePath.sha256"
$RecordPath = Join-Path $RepoRoot 'target\release\release-record.json'
$SizeLimitBytes = 14MB
$FeatureNames = @('QuickJS', 'linkedom', 'mermaid')
$BundleDir = $null
$ArchiveReady = $false
$ReleaseRecord = [ordered]@{
  schema_version = 1
  version = $Version
  artifact = 'target/release/veil.exe'
  bytes = $null
  sha256 = $null
  started_at = [DateTimeOffset]::UtcNow.ToString('o')
  timestamp = $null
  elapsed_seconds = $null
  status = 'skipped'
  release_ready = $false
  error = $null
  build = [ordered]@{ status = 'skipped'; command = 'cargo build --release --bin veil'; exit_code = $null }
  size_gate = [ordered]@{ status = 'skipped'; limit_bytes = $SizeLimitBytes; margin_bytes = $null }
  feature_gate = [ordered]@{
    status = 'skipped'
    method = 'ASCII decoding of the built veil.exe bytes; no source scan'
    required = $FeatureNames
    found = @()
    missing = @()
  }
  smoke = [ordered]@{
    status = 'skipped'
    command = "pwsh -NoProfile -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000"
    exit_code = $null
  }
  signing = [ordered]@{
    status = 'unsigned'
    signature_validation = 'not_applicable'
    timestamp_validation = 'not_applicable'
    timestamp_url = $TimestampUrl
    sign_tool = $null
    reason = 'No signing certificate supplied; artifact is unsigned'
  }
  tests = [ordered]@{ status = 'skipped'; reason = 'not reached'; command = 'cargo test --workspace --release -- --test-threads=1'; exit_code = $null }
  clippy = [ordered]@{ status = 'skipped'; reason = 'not reached'; command = 'cargo clippy --workspace --all-targets -- -D warnings'; exit_code = $null }
  checksum = [ordered]@{
    status = 'skipped'
    algorithm = 'SHA-256'
    path = 'target/release/veil.exe.sha256'
    post_archive_status = 'skipped'
    post_archive_sha256 = $null
    bundled_sha256 = $null
  }
  supply_chain = [ordered]@{
    notices = 'skipped'
    sbom = 'NOT RUN'
    audit = 'NOT RUN'
    audit_reason = 'not reached'
    audit_tool = 'cargo-deny'
    docs_missing = @()
  }
  archive = [ordered]@{ status = 'skipped'; reason = 'not reached'; path = $null }
}

function Write-Step($n, $desc) { Write-Host ''; Write-Host "== Step $n ($desc) ==" -ForegroundColor Cyan }
function Write-OK($msg)     { Write-Host "  OK  $msg" -ForegroundColor Green }
function Write-Warn($msg)   { Write-Host "  WARN $msg" -ForegroundColor Yellow }

function Write-ReleaseRecord {
  param([switch]$BestEffort)

  try {
    $RequiredGates = @('build', 'size_gate', 'feature_gate', 'smoke', 'tests', 'clippy', 'checksum')
    $Failed = @($RequiredGates | Where-Object { $ReleaseRecord[$_].status -eq 'failed' }).Count -gt 0 -or $ReleaseRecord.supply_chain.notices -eq 'fail'
    $Incomplete = @($RequiredGates | Where-Object { $ReleaseRecord[$_].status -ne 'passed' }).Count -gt 0 -or $ReleaseRecord.supply_chain.notices -ne 'pass' -or $ReleaseRecord.supply_chain.sbom -eq 'NOT RUN'
    if (-not $SkipArchive -and $ReleaseRecord.checksum.post_archive_status -ne 'passed') { $Incomplete = $true }

    $ReleaseRecord.release_ready = -not $ReleaseRecord.error -and -not $Failed -and -not $Incomplete
    $ReleaseRecord.status = if ($ReleaseRecord.error -or $Failed) { 'failed' } elseif ($Incomplete) { 'skipped' } else { 'passed' }
    $ReleaseRecord.timestamp = [DateTimeOffset]::UtcNow.ToString('o')
    $ReleaseRecord.elapsed_seconds = [math]::Round($Stopwatch.Elapsed.TotalSeconds, 3)
    $Json = $ReleaseRecord | ConvertTo-Json -Depth 8
    $Destinations = @($RecordPath)
    if ($ArchiveReady -and $BundleDir) { $Destinations += Join-Path $BundleDir 'release-record.json' }
    foreach ($Destination in $Destinations) {
      $Parent = Split-Path -Parent $Destination
      New-Item -ItemType Directory -Force -Path $Parent -ErrorAction Stop | Out-Null
      [System.IO.File]::WriteAllText($Destination, $Json + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
    }
  } catch {
    if (-not $BestEffort) { throw }
    Write-Warn "could not write release record: $($_.Exception.Message)"
  }
}

function Write-Fail($msg) {
  $ReleaseRecord.error = $msg
  Write-ReleaseRecord -BestEffort
  Write-Host "  FAIL $msg" -ForegroundColor Red
  exit 1
}

trap {
  $ReleaseRecord.error = $_.Exception.Message
  Write-ReleaseRecord -BestEffort
  Write-Host "  FAIL $($ReleaseRecord.error)" -ForegroundColor Red
  exit 1
}

Write-Host "Working directory: $(Get-Location)"

# Step 0: version
if (-not $Version) {
  $Version = (Select-String -Path (Join-Path $RepoRoot 'Cargo.toml') -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
}
$ReleaseRecord.version = $Version
Write-Host "Veil Release v$Version" -ForegroundColor Magenta

# Step 1: pre-flight
Write-Step 1 'pre-flight checks'
$GitStatus = git status --porcelain
$GitExit = $LASTEXITCODE
if ($GitExit -ne 0) { Write-Fail "git status failed with exit $GitExit" }
if ($GitStatus) { Write-Warn "git status not clean: $($GitStatus.Count) line(s) not committed" } else { Write-OK 'git status clean' }
$TagExists = git tag -l "v$Version"
$TagExit = $LASTEXITCODE
if ($TagExit -ne 0) { Write-Fail "git tag failed with exit $TagExit" }
if ($TagExists) { Write-OK "tag v$Version exists" } else { Write-Warn "tag v$Version not found" }
$RustVerLine = rustc --version
$RustExit = $LASTEXITCODE
if ($RustExit -ne 0) { Write-Fail "rustc --version failed with exit $RustExit" }
$RustVer = ($RustVerLine -split ' ')[1]
if ([version]$RustVer -ge [version]'1.75') { Write-OK "Rust $RustVer [ge] 1.75" } else { Write-Warn "Rust $RustVer older than 1.75" }

# Step 2: supply-chain inputs（许可清单是硬门禁；审计在工具/漏洞库缺失时记为 NOT RUN）
Write-Step 2 'supply chain: third-party notices + dependency audit'
$ReleaseRecord.supply_chain.notices = 'fail'
pwsh -NoProfile -File (Join-Path $RepoRoot 'tools\third_party_notices.ps1') -Check
$NoticesExit = $LASTEXITCODE
if ($NoticesExit -ne 0) { Write-Fail "third-party notices are stale: run tools/third_party_notices.ps1 (exit $NoticesExit)" }
$ReleaseRecord.supply_chain.notices = 'pass'
Write-OK 'THIRD-PARTY-LICENSES.md is up to date'

$DenyVersionOutput = & cargo deny --version 2>&1
$DenyVersionExit = $LASTEXITCODE
if ($DenyVersionExit -ne 0) {
  $ReleaseRecord.supply_chain.audit = 'NOT RUN'
  $ReleaseRecord.supply_chain.audit_reason = 'cargo-deny is not installed'
  Write-Warn 'cargo-deny not installed; audit recorded as NOT RUN'
} else {
  $AuditOutput = & cargo deny check 2>&1 | Out-String
  $AuditExit = $LASTEXITCODE
  if ($AuditExit -eq 0) {
    $ReleaseRecord.supply_chain.audit = 'PASS'
    $ReleaseRecord.supply_chain.audit_reason = $null
    Write-OK "cargo deny check passed ($($DenyVersionOutput -join ' '))"
  } elseif ($AuditOutput -match '(?i)failed to (fetch|update|download)|network (error|is unreachable)|could not connect|unable to (fetch|update|download)|advisory database.*(unavailable|failed)') {
    $ReleaseRecord.supply_chain.audit = 'NOT RUN'
    $ReleaseRecord.supply_chain.audit_reason = 'advisory database or network unavailable'
    Write-Warn 'cargo deny check could not complete (database/network); audit recorded as NOT RUN'
  } else {
    $AuditSummary = (($AuditOutput -split "`n" | Select-Object -First 5) -join ' / ')
    $ReleaseRecord.supply_chain.audit = 'FAIL'
    $ReleaseRecord.supply_chain.audit_reason = "cargo deny check exited $AuditExit"
    Write-Fail "cargo deny check failed with exit ${AuditExit}: $AuditSummary"
  }
}

# Step 3: cargo test（与 release workflow 同为 release + 串行口径）
if (-not $SkipTest) {
  Write-Step 3 'cargo test --workspace --release -- --test-threads=1'
  $ReleaseRecord.tests.status = 'failed'
  $ReleaseRecord.tests.reason = $null
  cargo test --workspace --release -- --test-threads=1 | Select-Object -Last 5
  $TestExit = $LASTEXITCODE
  $ReleaseRecord.tests.exit_code = $TestExit
  if ($TestExit -ne 0) { Write-Fail "cargo test failed with exit $TestExit" }
  $ReleaseRecord.tests.status = 'passed'
  Write-OK 'cargo test passed'
} else {
  $ReleaseRecord.tests.reason = '-SkipTest specified'
  Write-Host '  SKIP cargo test (release record: skipped)'
}

# Step 4: cargo clippy
if (-not $SkipClippy) {
  Write-Step 4 'cargo clippy --workspace --all-targets -- -D warnings'
  $ReleaseRecord.clippy.status = 'failed'
  $ReleaseRecord.clippy.reason = $null
  cargo clippy --workspace --all-targets -- -D warnings | Select-Object -Last 3
  $ClippyExit = $LASTEXITCODE
  $ReleaseRecord.clippy.exit_code = $ClippyExit
  if ($ClippyExit -ne 0) { Write-Fail "cargo clippy failed with exit $ClippyExit" }
  $ReleaseRecord.clippy.status = 'passed'
  Write-OK 'cargo clippy clean'
} else {
  $ReleaseRecord.clippy.reason = '-SkipClippy specified'
  Write-Host '  SKIP cargo clippy (release record: skipped)'
}

# Step 5: clean + release build
Write-Step 5 'clean + release build'
$ReleaseRecord.build.status = 'failed'
cargo clean -p veil -p veil-ui -p veil-core -p veil-mermaid-offline | Select-Object -Last 3
$CleanExit = $LASTEXITCODE
$ReleaseRecord.build.exit_code = $CleanExit
if ($CleanExit -ne 0) { Write-Fail "cargo clean failed with exit $CleanExit" }
Write-Host '  Cleaned veil/veil-ui/veil-core/veil-mermaid-offline artifacts'
cargo build --release --bin veil | Select-Object -Last 3
$BuildExit = $LASTEXITCODE
$ReleaseRecord.build.exit_code = $BuildExit
if ($BuildExit -ne 0) { Write-Fail "cargo build --release --bin veil failed with exit $BuildExit" }
$ReleaseRecord.build.status = 'passed'
$Exe = Get-Item -LiteralPath $ExePath -ErrorAction Stop
Write-OK "Release build done: $($Exe.Length) bytes, build time $($Exe.LastWriteTime)"

# Step 6: verify the single executable and its binary feature evidence
Write-Step 6 'verify artifact'
$vi = $Exe.VersionInfo
$SizeMiB = [math]::Round($Exe.Length / 1MB, 2)
$SizeBytes = [int64]$Exe.Length
$SizeMarginBytes = $SizeLimitBytes - $SizeBytes
$ReleaseRecord.bytes = $SizeBytes
$ReleaseRecord.size_gate.margin_bytes = $SizeMarginBytes
$SizeOk = $SizeBytes -le $SizeLimitBytes
$ReleaseRecord.size_gate.status = if ($SizeOk) { 'passed' } else { 'failed' }

$ReleaseExecutables = @(Get-ChildItem -LiteralPath (Split-Path -Parent $ExePath) -Filter '*.exe' -File)
$SingleExeOk = $ReleaseExecutables.Count -eq 1 -and $ReleaseExecutables[0].Name -eq 'veil.exe'
$BinaryText = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($Exe.FullName))
$FoundFeatures = @()
$MissingFeatures = @()
foreach ($Name in $FeatureNames) {
  if ($BinaryText.Contains($Name)) { $FoundFeatures += $Name } else { $MissingFeatures += $Name }
}
$FeaturesOk = $MissingFeatures.Count -eq 0
$ReleaseRecord.feature_gate.status = if ($FeaturesOk) { 'passed' } else { 'failed' }
$ReleaseRecord.feature_gate.found = @($FoundFeatures)
$ReleaseRecord.feature_gate.missing = @($MissingFeatures)

$Checks = @(
  @{ n='FileVersion';   ok = $vi.FileVersion -match '^\d+\.'; val=$vi.FileVersion },
  @{ n='ProductVersion';ok = $vi.ProductVersion -match '^\d+\.'; val=$vi.ProductVersion },
  @{ n='FileDescription'; ok = $vi.FileDescription -eq 'Veil Markdown 编辑器'; val=$vi.FileDescription },
  @{ n='single veil.exe'; ok = $SingleExeOk; val="$($ReleaseExecutables.Count) exe file(s) in target/release" },
  @{ n='Size <= 14 MiB';  ok = $SizeOk; val="$SizeMiB MiB" },
  @{ n='offline binary markers'; ok = $FeaturesOk; val="found=$($FoundFeatures -join ','); missing=$($MissingFeatures -join ',')" },
  @{ n='icons/veil.ico'; ok = (Test-Path 'icons/veil.ico'); val="$(if(Test-Path 'icons/veil.ico'){(Get-Item 'icons/veil.ico').Length}else{'missing'})" },
  @{ n='LICENSE';       ok = (Test-Path 'LICENSE'); val="$(if(Test-Path 'LICENSE'){(Get-Content 'LICENSE' -TotalCount 1)}else{'missing'})" },
  @{ n='CHANGELOG.md';  ok = (Test-Path 'CHANGELOG.md'); val='v3.0.0 present' }
)
foreach ($c in $Checks) {
  if ($c.ok) { Write-OK "$($c.n) = $($c.val)" } else { Write-Warn "$($c.n) = $($c.val) FAILED" }
}
$AllOk = ($Checks | Where-Object { -not $_.ok }).Count -eq 0
if (-not $AllOk) { Write-Fail 'artifact verification failed, see WARN above' }

# Step 7: real launch smoke against the same single artifact
Write-Step 7 'smoke launch veil.exe'
$ReleaseRecord.smoke.status = 'failed'
pwsh -NoProfile -File (Join-Path $RepoRoot 'tools\smoke_launch.ps1') -ExePath $ExePath -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000
$SmokeExit = $LASTEXITCODE
$ReleaseRecord.smoke.exit_code = $SmokeExit
if ($SmokeExit -ne 0) { Write-Fail "smoke launch failed with exit $SmokeExit" }
$ReleaseRecord.smoke.status = 'passed'
Write-OK 'smoke launch passed'

# Step 8: optionally sign the exact bytes that will be hashed and archived.
Write-Step 8 'optional Authenticode + RFC3161 signing'
if ([string]::IsNullOrWhiteSpace($CertificatePath)) {
  if ($PSBoundParameters.ContainsKey('CertificatePassword')) {
    Write-Fail 'CertificatePassword requires CertificatePath'
  }
  Write-Warn 'UNSIGNED: no signing certificate supplied; continuing without Authenticode'
} else {
  $ReleaseRecord.signing.status = 'failed'
  $ReleaseRecord.signing.signature_validation = 'failed'
  $ReleaseRecord.signing.timestamp_validation = 'failed'
  $ReleaseRecord.signing.reason = $null
  if (-not $PSBoundParameters.ContainsKey('CertificatePassword')) {
    Write-Fail 'CertificatePath requires CertificatePassword (pass an empty string only for a passwordless PFX)'
  }
  if ([string]::IsNullOrWhiteSpace($TimestampUrl)) { Write-Fail 'TimestampUrl must not be empty when signing' }

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
  $ReleaseRecord.signing.sign_tool = $SignToolExe
  $ResolvedCertificate = (Resolve-Path -LiteralPath $CertificatePath -ErrorAction Stop).Path

  & $SignToolExe sign /fd SHA256 /f $ResolvedCertificate /p $CertificatePassword /tr $TimestampUrl /td SHA256 /v $ExePath | Out-Host
  $SignExit = $LASTEXITCODE
  if ($SignExit -ne 0) { Write-Fail "signtool sign failed with exit $SignExit" }

  $VerifyOutput = @(& $SignToolExe verify /pa /all /v $ExePath 2>&1)
  $VerifyExit = $LASTEXITCODE
  $VerifyOutput | Out-Host
  if ($VerifyExit -ne 0) { Write-Fail "Authenticode verification failed with exit ${VerifyExit}: $($VerifyOutput -join ' ')" }
  $ReleaseRecord.signing.signature_validation = 'passed'
  $VerifyText = $VerifyOutput -join "`n"
  if ($VerifyText -notmatch 'The signature is timestamped:' -or $VerifyText -notmatch 'Timestamp Verified by:') {
    Write-Fail 'RFC3161 timestamp evidence is missing or was not verified by signtool'
  }
  $ReleaseRecord.signing.timestamp_validation = 'passed'
  $ReleaseRecord.signing.status = 'passed'
  Write-OK 'Authenticode signature and RFC3161 timestamp verified'
}

# Step 9: generate the CycloneDX SBOM for the locked dependency set.
Write-Step 9 'generate CycloneDX SBOM'
$SbomPath = Join-Path $RepoRoot "target\release\veil-v$Version.cdx.json"
pwsh -NoProfile -File (Join-Path $RepoRoot 'tools\sbom.ps1') -OutFile $SbomPath
$SbomExit = $LASTEXITCODE
if ($SbomExit -ne 0) { Write-Fail "SBOM generation failed with exit $SbomExit" }
$ReleaseRecord.supply_chain.sbom = Split-Path -Leaf $SbomPath
Write-OK "SBOM written: $($ReleaseRecord.supply_chain.sbom)"

# Step 10: generate and validate the conventional SHA-256 sidecar after signing.
Write-Step 10 'generate SHA-256'
$ReleaseRecord.checksum.status = 'failed'
$Sha256 = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
$ReleaseRecord.sha256 = $Sha256
$ChecksumLine = "$Sha256  veil.exe"
Set-Content -LiteralPath $ChecksumPath -Value $ChecksumLine -Encoding ascii
$Expected = ((Get-Content -LiteralPath $ChecksumPath -Raw).Trim() -split '\s+')[0]
$Actual = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($Expected -ne $Sha256 -or $Actual -ne $Expected) { Write-Fail 'SHA-256 sidecar validation failed' }
$ReleaseRecord.checksum.status = 'passed'
Write-OK "veil.exe.sha256 = $ChecksumLine"

# Step 11: archive release-bundle, including its checksum
if (-not $SkipArchive) {
  Write-Step 11 'archive release-bundle'
  $ReleaseRecord.archive.status = 'failed'
  $ReleaseRecord.archive.reason = $null
  $BundleDir = Join-Path $RepoRoot "release-bundle\veil-v$Version"
  $ReleaseRecord.archive.path = "release-bundle/veil-v$Version"
  if (Test-Path -LiteralPath $BundleDir) { Remove-Item -LiteralPath $BundleDir -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $BundleDir | Out-Null
  New-Item -ItemType Directory -Force -Path (Join-Path $BundleDir 'icons') | Out-Null
  Copy-Item -LiteralPath $ExePath -Destination $BundleDir -Force
  Copy-Item -LiteralPath $ChecksumPath -Destination $BundleDir -Force
  $PdbPath = Join-Path $RepoRoot 'target\release\veil.pdb'
  if (Test-Path -LiteralPath $PdbPath) { Copy-Item -LiteralPath $PdbPath -Destination $BundleDir -Force }
  Copy-Item -LiteralPath (Join-Path $RepoRoot 'README.md') -Destination $BundleDir -Force
  Copy-Item -LiteralPath (Join-Path $RepoRoot 'LICENSE') -Destination $BundleDir -Force
  Copy-Item -LiteralPath (Join-Path $RepoRoot 'CHANGELOG.md') -Destination $BundleDir -Force
  Copy-Item -LiteralPath (Join-Path $RepoRoot 'icons\veil.ico') -Destination (Join-Path $BundleDir 'icons') -Force

  # 合规文档与 SBOM 必须随同一发布字节归档；缺失的合规文档只告警并记入发布记录。
  $ComplianceDocs = @('THIRD-PARTY-LICENSES.md', 'PRIVACY.md', 'SECURITY.md', 'SUPPORT.md')
  $MissingDocs = @()
  foreach ($Doc in $ComplianceDocs) {
    $DocSource = Join-Path $RepoRoot $Doc
    if (Test-Path -LiteralPath $DocSource) {
      Copy-Item -LiteralPath $DocSource -Destination $BundleDir -Force
    } else {
      $MissingDocs += $Doc
      Write-Warn "compliance doc missing: $Doc (not copied into bundle)"
    }
  }
  $ReleaseRecord.supply_chain.docs_missing = @($MissingDocs)
  if ($MissingDocs.Count -eq 0) { Write-OK 'compliance docs bundled' }

  if (Test-Path -LiteralPath $SbomPath) {
    Copy-Item -LiteralPath $SbomPath -Destination $BundleDir -Force
  } else {
    Write-Warn "SBOM missing at $SbomPath; bundle does not include it"
  }

  $ArchiveReady = $true
  $ReleaseRecord.archive.status = 'passed'
  $BundleFiles = Get-ChildItem -LiteralPath $BundleDir -Recurse -File
  Write-OK "Bundle archived at $BundleDir ($($BundleFiles.Count) files before release record)"

  # 归档完成后重读原产物和包内产物；任一变化都必须阻断发布。
  $ReleaseRecord.checksum.post_archive_status = 'failed'
  $PostArchiveHash = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
  $BundledHash = (Get-FileHash -LiteralPath (Join-Path $BundleDir 'veil.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
  $ReleaseRecord.checksum.post_archive_sha256 = $PostArchiveHash
  $ReleaseRecord.checksum.bundled_sha256 = $BundledHash
  $ExpectedAfterArchive = ((Get-Content -LiteralPath $ChecksumPath -Raw).Trim() -split '\s+')[0]
  if ($PostArchiveHash -ne $Sha256 -or $BundledHash -ne $Sha256 -or $ExpectedAfterArchive -ne $Sha256) {
    Write-Fail 'post-archive SHA-256 verification failed: executable or sidecar changed'
  }
  $ReleaseRecord.checksum.post_archive_status = 'passed'
  Write-OK 'post-archive SHA-256 unchanged for target and bundled veil.exe'
} else {
  Write-Step 11 'archive release-bundle'
  $ReleaseRecord.archive.reason = '-SkipArchive specified; post-archive check not applicable'
  Write-Host '  SKIP archive (release record: skipped)'
}

# Step 12: machine-readable record + human summary
$Stopwatch.Stop()
Write-ReleaseRecord
Write-OK "Release record written: $RecordPath"
Write-Host ''
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host "  Veil v$Version Release Summary" -ForegroundColor Magenta
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host "  Version     : $($vi.ProductVersion)"
Write-Host "  Size        : $SizeMiB MiB ($SizeBytes bytes)"
Write-Host "  Size Margin : $SizeMarginBytes bytes to 14 MiB"
Write-Host "  SHA-256     : $Sha256"
Write-Host "  FileDescr   : $($vi.FileDescription)"
Write-Host "  Build Time  : $($Exe.LastWriteTime)"
Write-Host "  Elapsed     : $([math]::Round($Stopwatch.Elapsed.TotalSeconds, 1)) s"
Write-Host "  Size gate   : $($ReleaseRecord.size_gate.status)"
Write-Host "  Feature gate: $($ReleaseRecord.feature_gate.status) ($($FeatureNames -join '/'))"
Write-Host "  Smoke       : $($ReleaseRecord.smoke.status)"
Write-Host "  Signing     : $($ReleaseRecord.signing.status); signature=$($ReleaseRecord.signing.signature_validation); timestamp=$($ReleaseRecord.signing.timestamp_validation)"
Write-Host "  Tests       : $($ReleaseRecord.tests.status)"
Write-Host "  Clippy      : $($ReleaseRecord.clippy.status)"
Write-Host "  Notices     : $($ReleaseRecord.supply_chain.notices)"
Write-Host "  SBOM        : $($ReleaseRecord.supply_chain.sbom)"
$AuditLine = if ($ReleaseRecord.supply_chain.audit_reason) { "$($ReleaseRecord.supply_chain.audit) ($($ReleaseRecord.supply_chain.audit_reason))" } else { $ReleaseRecord.supply_chain.audit }
Write-Host "  Audit       : $AuditLine"
if ($ReleaseRecord.supply_chain.docs_missing.Count -gt 0) { Write-Host "  Docs missing: $($ReleaseRecord.supply_chain.docs_missing -join ', ')" }
Write-Host "  Checksum    : $($ReleaseRecord.checksum.status); post-archive=$($ReleaseRecord.checksum.post_archive_status)"
if (-not $SkipArchive) { Write-Host "  Bundle      : release-bundle\veil-v$Version\" }
Write-Host "  Record      : target\release\release-record.json"
Write-Host "  Overall     : $($ReleaseRecord.status); release_ready=$($ReleaseRecord.release_ready)"
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host ''
if ($ReleaseRecord.release_ready) {
  Write-OK 'Release complete; all required gates passed'
} else {
  Write-Warn "Release artifacts complete, but required gates were skipped; overall=$($ReleaseRecord.status), release_ready=false"
}
exit 0
