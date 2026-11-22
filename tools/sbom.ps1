[CmdletBinding()]
param(
  [string]$OutFile
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$RepoRoot = Split-Path -Parent $PSScriptRoot
$LockPath = Join-Path $RepoRoot 'Cargo.lock'
$ManifestPath = Join-Path $RepoRoot 'Cargo.toml'

if (-not (Test-Path -LiteralPath $LockPath)) { throw "Cargo.lock not found at $LockPath" }

# Workspace version, used for the default output name and metadata.component.
$Version = $null
$versionMatch = Select-String -Path $ManifestPath -Pattern '^version = "([^"]+)"' | Select-Object -First 1
if ($versionMatch) { $Version = $versionMatch.Matches.Groups[1].Value }
if ([string]::IsNullOrWhiteSpace($Version)) { throw 'could not determine workspace version from Cargo.toml' }

if ([string]::IsNullOrWhiteSpace($OutFile)) {
  $OutFile = Join-Path $RepoRoot "target\release\veil-v$Version.cdx.json"
} elseif (-not [System.IO.Path]::IsPathRooted($OutFile)) {
  $OutFile = Join-Path $RepoRoot $OutFile
}

function Read-LockPackages {
  param([string]$Path)

  $packages = New-Object System.Collections.Generic.List[object]
  $current = $null

  foreach ($line in [System.IO.File]::ReadLines($Path)) {
    if ($line -match '^\[\[package\]\]\s*$') {
      if ($current) { $packages.Add($current) }
      $current = [ordered]@{ name = $null; version = $null; source = $null; checksum = $null }
      continue
    }
    if ($line -match '^\[') {
      if ($current) { $packages.Add($current) }
      $current = $null
      continue
    }
    if ($current -and $line -match '^([A-Za-z0-9_-]+) = "(.*)"\s*$') {
      $key = $Matches[1]
      if ($current.Contains($key)) { $current[$key] = $Matches[2] }
    }
  }
  if ($current) { $packages.Add($current) }

  return $packages
}

function Get-MetadataLicenses {
  $raw = & cargo metadata --locked --format-version 1 --manifest-path $ManifestPath 2>$null | Out-String
  if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed with exit $LASTEXITCODE" }
  $metadata = $raw | ConvertFrom-Json

  $map = @{}
  foreach ($package in $metadata.packages) {
    $map["$($package.name)@$($package.version)"] = $package.license
  }
  return [pscustomobject]@{
    Licenses = $map
    WorkspaceMembers = @($metadata.workspace_members)
    WorkspaceIdsByName = @($metadata.packages | Where-Object { $metadata.workspace_members -contains $_.id } | ForEach-Object { "$($_.name)@$($_.version)" })
  }
}

function New-LicenseEntries {
  param([string]$Expression, [bool]$IsWorkspaceMember)

  if ([string]::IsNullOrWhiteSpace($Expression)) {
    if ($IsWorkspaceMember) {
      return @([ordered]@{ license = [ordered]@{ id = 'MIT' } })
    }
    return @([ordered]@{ license = [ordered]@{ name = 'UNSPECIFIED' } })
  }

  if ($Expression -match '^[A-Za-z0-9][A-Za-z0-9.+-]*$') {
    return @([ordered]@{ license = [ordered]@{ id = $Expression } })
  }

  return @([ordered]@{ license = [ordered]@{ name = $Expression } })
}

$lockPackages = Read-LockPackages -Path $LockPath
$metadataLicenses = Get-MetadataLicenses

# Ordinal ordering keeps the SBOM byte-identical across machines.
$sortedPackages = @($lockPackages)
$comparison = [System.Comparison[object]] {
  param($Left, $Right)
  $result = [string]::CompareOrdinal($Left.name, $Right.name)
  if ($result -ne 0) { return $result }
  return [string]::CompareOrdinal($Left.version, $Right.version)
}
[Array]::Sort($sortedPackages, $comparison)

$components = New-Object System.Collections.Generic.List[object]
foreach ($package in $sortedPackages) {
  $key = "$($package.name)@$($package.version)"
  $license = $metadataLicenses.Licenses[$key]
  $isWorkspaceMember = $metadataLicenses.WorkspaceIdsByName -contains $key

  $components.Add([ordered]@{
    type = 'library'
    name = $package.name
    version = $package.version
    purl = "pkg:cargo/$($package.name)@$($package.version)"
    licenses = @(New-LicenseEntries -Expression $license -IsWorkspaceMember $isWorkspaceMember)
  })
}

$document = [ordered]@{
  bomFormat = 'CycloneDX'
  specVersion = '1.5'
  version = 1
  metadata = [ordered]@{
    component = [ordered]@{
      type = 'application'
      name = 'Veil'
      version = $Version
    }
  }
  components = $components.ToArray()
}

$json = ($document | ConvertTo-Json -Depth 10) -replace "`r`n", "`n"
$json = $json.TrimEnd("`n") + "`n"

$parent = Split-Path -Parent $OutFile
if (-not [string]::IsNullOrWhiteSpace($parent)) {
  New-Item -ItemType Directory -Force -Path $parent | Out-Null
}
[System.IO.File]::WriteAllText($OutFile, $json, [System.Text.UTF8Encoding]::new($false))

# Self-check: the emitted file must parse and cover every Cargo.lock package.
$written = [System.IO.File]::ReadAllText($OutFile, [System.Text.Encoding]::UTF8)
$parsed = $written | ConvertFrom-Json
if ($parsed.bomFormat -ne 'CycloneDX' -or $parsed.specVersion -ne '1.5') {
  throw 'SBOM self-check failed: bomFormat/specVersion mismatch'
}
if ($null -eq $parsed.components -or $parsed.components.Count -ne $lockPackages.Count) {
  throw "SBOM self-check failed: component count $($parsed.components.Count) != Cargo.lock package count $($lockPackages.Count)"
}

$hash = (Get-FileHash -LiteralPath $OutFile -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "Wrote $OutFile" -ForegroundColor Green
Write-Host ("  components: {0} (Cargo.lock packages: {1})" -f $parsed.components.Count, $lockPackages.Count)
Write-Host ("  SHA-256: {0}" -f $hash)
exit 0
