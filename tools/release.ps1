param(
  [switch]$SkipTest,
  [switch]$SkipClippy,
  [switch]$SkipArchive,
  [string]$Version
)

$ErrorActionPreference = 'Continue'
$Stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

function Write-Step($n, $desc) { Write-Host ''; Write-Host "== Step $n ($desc) ==" -ForegroundColor Cyan }
function Write-OK($msg)     { Write-Host "  OK  $msg" -ForegroundColor Green }
function Write-Warn($msg)   { Write-Host "  WARN $msg" -ForegroundColor Yellow }
function Write-Fail($msg)   { Write-Host "  FAIL $msg" -ForegroundColor Red; exit 1 }

Write-Host "Working directory: $(Get-Location)"

# Step 0: version
if (-not $Version) {
  $Version = (Select-String -Path (Join-Path $RepoRoot 'Cargo.toml') -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
}
Write-Host "Veil Release v$Version" -ForegroundColor Magenta

# Step 1: pre-flight
Write-Step 1 'pre-flight checks'
$GitStatus = git status --porcelain
if ($GitStatus) { Write-Warn "git status not clean: $($GitStatus.Count) line(s) not committed" } else { Write-OK 'git status clean' }
$TagExists = git tag -l "v$Version"
if ($TagExists) { Write-OK "tag v$Version exists" } else { Write-Warn "tag v$Version not found" }
$RustVerLine = rustc --version
$RustVer = ($RustVerLine -split ' ')[1]
if ([version]$RustVer -ge [version]'1.75') { Write-OK "Rust $RustVer [ge] 1.75" } else { Write-Warn "Rust $RustVer older than 1.75" }

# Step 2: cargo test
if (-not $SkipTest) {
  Write-Step 2 'cargo test --workspace'
  cargo test --workspace 2>&1 | Select-Object -Last 5
  if ($LASTEXITCODE -ne 0) { Write-Fail "cargo test failed with exit $LASTEXITCODE" }
  Write-OK 'cargo test passed'
} else { Write-Host '  SKIP cargo test' }

# Step 3: cargo clippy
if (-not $SkipClippy) {
  Write-Step 3 'cargo clippy'
  cargo clippy --workspace --all-targets -- -D warnings 2>&1 | Select-Object -Last 3
  if ($LASTEXITCODE -ne 0) { Write-Fail 'cargo clippy failed' }
  Write-OK 'cargo clippy clean'
} else { Write-Host '  SKIP cargo clippy' }

# Step 4: clean + release build
Write-Step 4 'clean + release build'
cargo clean -p veil -p veil-ui -p veil-core --release 2>$null
Write-Host '  Cleaned veil/veil-ui/veil-core release artifacts'
cargo build --release -p veil 2>&1 | Select-Object -Last 3
if ($LASTEXITCODE -ne 0) { Write-Fail 'cargo build --release failed' }
$Exe = Get-Item target\release\veil.exe
Write-OK "Release build done: $($Exe.Length) bytes, build time $($Exe.LastWriteTime)"

# Step 5: verify artifact
Write-Step 5 'verify artifact'
$vi = $Exe.VersionInfo
$SizeMiB = [math]::Round($Exe.Length / 1MB, 2)
$Checks = @(
  @{ n='FileVersion';   ok = $vi.FileVersion -match '^1\.0\.0'; val=$vi.FileVersion },
  @{ n='ProductVersion';ok = $vi.ProductVersion -match '^1\.0\.0'; val=$vi.ProductVersion },
  @{ n='FileDescription'; ok = $vi.FileDescription -eq 'Veil Markdown 编辑器'; val=$vi.FileDescription },
  @{ n='Size 4-8 MiB';  ok = ($Exe.Length -ge 4MB) -and ($Exe.Length -le 8MB); val="$SizeMiB MiB" },
  @{ n='icons/veil.ico'; ok = (Test-Path 'icons/veil.ico'); val="$(if(Test-Path 'icons/veil.ico'){(Get-Item 'icons/veil.ico').Length}else{'missing'})" },
  @{ n='LICENSE';       ok = (Test-Path 'LICENSE'); val="$(if(Test-Path 'LICENSE'){(Get-Content 'LICENSE' -TotalCount 1)}else{'missing'})" },
  @{ n='CHANGELOG.md';  ok = (Test-Path 'CHANGELOG.md'); val='v1.0.0 present' }
)
foreach ($c in $Checks) {
  if ($c.ok) { Write-OK "$($c.n) = $($c.val)" } else { Write-Warn "$($c.n) = $($c.val) FAILED" }
}
$AllOk = ($Checks | Where-Object { -not $_.ok }).Count -eq 0
if (-not $AllOk) { Write-Fail 'artifact verification failed, see WARN above' }

# Step 6: archive release-bundle
if (-not $SkipArchive) {
  Write-Step 6 'archive release-bundle'
  $BundleDir = "release-bundle\veil-v$Version"
  if (Test-Path $BundleDir) { Remove-Item -Recurse -Force $BundleDir }
  New-Item -ItemType Directory -Force -Path $BundleDir | Out-Null
  New-Item -ItemType Directory -Force -Path "$BundleDir\icons" | Out-Null
  Copy-Item target\release\veil.exe $BundleDir\ -Force
  if (Test-Path target\release\veil.pdb) { Copy-Item target\release\veil.pdb $BundleDir\ -Force }
  Copy-Item README.md $BundleDir\ -Force
  Copy-Item LICENSE $BundleDir\ -Force
  Copy-Item CHANGELOG.md $BundleDir\ -Force
  Copy-Item icons\veil.ico "$BundleDir\icons\" -Force
  $BundleFiles = Get-ChildItem $BundleDir -Recurse
  Write-OK "Bundle archived at $BundleDir ($($BundleFiles.Count) files)"
} else { Write-Host '  SKIP archive' }

# Step 7: summary
$Stopwatch.Stop()
Write-Host ''
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host "  Veil v$Version Release Summary" -ForegroundColor Magenta
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host "  Version     : $($vi.ProductVersion)"
Write-Host "  Size        : $SizeMiB MiB"
Write-Host "  FileDescr   : $($vi.FileDescription)"
Write-Host "  Build Time  : $($Exe.LastWriteTime)"
Write-Host "  Elapsed     : $([math]::Round($Stopwatch.Elapsed.TotalSeconds, 1)) s"
if (-not $SkipTest)   { Write-Host '  Tests       : passed' } else { Write-Host '  Tests       : SKIPPED' }
if (-not $SkipClippy) { Write-Host '  Clippy      : clean' } else { Write-Host '  Clippy      : SKIPPED' }
if (-not $SkipArchive) { Write-Host "  Bundle      : release-bundle\veil-v$Version\" }
Write-Host '=====================================================' -ForegroundColor Magenta
Write-Host ''
Write-OK 'Release complete'
exit 0
