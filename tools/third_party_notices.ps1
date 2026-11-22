[CmdletBinding()]
param(
  [switch]$Check,
  [string]$OutFile
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ManifestPath = Join-Path $RepoRoot 'Cargo.toml'
$AssetDir = Join-Path $RepoRoot 'crates\veil-mermaid-offline\assets'
$RootLicense = Join-Path $RepoRoot 'LICENSE'

if ([string]::IsNullOrWhiteSpace($OutFile)) {
  $OutFile = Join-Path $RepoRoot 'THIRD-PARTY-LICENSES.md'
} elseif (-not [System.IO.Path]::IsPathRooted($OutFile)) {
  $OutFile = Join-Path $RepoRoot $OutFile
}

function Invoke-CargoMetadata {
  $raw = & cargo metadata --locked --format-version 1 --manifest-path $ManifestPath 2>$null | Out-String
  if ($LASTEXITCODE -ne 0) { throw "cargo metadata --locked failed with exit $LASTEXITCODE" }
  return ($raw | ConvertFrom-Json)
}

function Sort-Packages {
  param($Items)

  # Ordinal ordering keeps the generated file byte-identical across machines.
  $array = @($Items)
  $comparison = [System.Comparison[object]] {
    param($Left, $Right)
    $result = [string]::CompareOrdinal($Left.name, $Right.name)
    if ($result -ne 0) { return $result }
    return [string]::CompareOrdinal($Left.version, $Right.version)
  }
  [Array]::Sort($array, $comparison)
  return $array
}

function Get-NormalizedText {
  param([string]$Path)
  $text = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
  $text = $text.TrimStart([char]0xFEFF)
  $text = $text -replace "`r`n", "`n" -replace "`r", "`n"
  return $text.TrimEnd() + "`n"
}

function Get-TextHash {
  param([string]$Text)
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($Text))
    return ([System.BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

function Format-Cell {
  param([string]$Value)
  if ([string]::IsNullOrWhiteSpace($Value)) { return '-' }
  return $Value.Replace('|', '\|').Replace("`r", ' ').Replace("`n", ' ')
}

function Find-LicenseFiles {
  param($Package, [bool]$IsWorkspaceMember)

  $dir = Split-Path -Parent $Package.manifest_path
  $found = @()
  if (Test-Path -LiteralPath $dir) {
    $found = @(Get-ChildItem -LiteralPath $dir -File -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -match '^(?i)(LICENSE|COPYING|NOTICE)' })
    if (-not [string]::IsNullOrWhiteSpace($Package.license_file)) {
      $declared = if ([System.IO.Path]::IsPathRooted($Package.license_file)) {
        $Package.license_file
      } else {
        Join-Path $dir $Package.license_file
      }
      if (Test-Path -LiteralPath $declared) { $found += Get-Item -LiteralPath $declared }
    }
  }

  $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
  $unique = @()
  foreach ($file in ($found | Sort-Object -Property Name)) {
    if ($seen.Add($file.Name)) { $unique += $file }
  }

  if ($unique.Count -eq 0 -and $IsWorkspaceMember -and (Test-Path -LiteralPath $RootLicense)) {
    return @([pscustomobject]@{ File = (Get-Item -LiteralPath $RootLicense); Label = 'LICENSE (workspace root)' })
  }

  return @($unique | ForEach-Object { [pscustomobject]@{ File = $_; Label = $_.Name } })
}

# ---------------------------------------------------------------------------
# Collect data
# ---------------------------------------------------------------------------

$Metadata = Invoke-CargoMetadata
$Packages = Sort-Packages $Metadata.packages
$WorkspaceIds = @($Metadata.workspace_members)
$WorkspaceVersion = '0.0.0'
$workspacePackage = $Metadata.packages | Where-Object { $_.name -eq 'veil' } | Select-Object -First 1
if ($workspacePackage) {
  $WorkspaceVersion = $workspacePackage.version
} else {
  $match = Select-String -Path $ManifestPath -Pattern '^version = "([^"]+)"' | Select-Object -First 1
  if ($match) { $WorkspaceVersion = $match.Matches.Groups[1].Value }
}

$Sections = @{}
$NoText = New-Object System.Collections.Generic.List[object]

foreach ($package in $Packages) {
  $isWorkspaceMember = $WorkspaceIds -contains $package.id
  $licenseFiles = Find-LicenseFiles -Package $package -IsWorkspaceMember $isWorkspaceMember

  if ($licenseFiles.Count -eq 0) {
    $expression = if ([string]::IsNullOrWhiteSpace($package.license)) {
      if ($isWorkspaceMember) { 'MIT (workspace)' } else { 'UNSPECIFIED' }
    } else {
      $package.license
    }
    $NoText.Add([pscustomobject]@{
      Name = $package.name
      Version = $package.version
      License = $expression
    })
    continue
  }

  foreach ($entry in $licenseFiles) {
    $content = Get-NormalizedText -Path $entry.File.FullName
    $hash = Get-TextHash -Text $content
    if (-not $Sections.ContainsKey($hash)) {
      $Sections[$hash] = [pscustomobject]@{
        Content = $content
        Entries = New-Object System.Collections.Generic.List[string]
      }
    }
    $Sections[$hash].Entries.Add(("``{0}`` {1} (``{2}``)" -f $package.name, $package.version, $entry.Label))
  }
}

# Embedded asset metadata (versions are read from the packaged files, never guessed).
$MermaidVersion = 'bundled'
$MermaidVersionNote = '打包文件未记录版本号'
$mermaidPath = Join-Path $AssetDir 'mermaid.min.js'
if (Test-Path -LiteralPath $mermaidPath) {
  $mermaidText = [System.IO.File]::ReadAllText($mermaidPath, [System.Text.Encoding]::UTF8)
  $versionMatch = [regex]::Match($mermaidText, 'name:"mermaid",version:"([^"]+)"')
  if ($versionMatch.Success) {
    $MermaidVersion = $versionMatch.Groups[1].Value
    $MermaidVersionNote = '取自打包文件内的 `name:"mermaid",version:"..."` 字段'
  }
}

$QuickJsPackage = $Packages | Where-Object { $_.name -eq 'rquickjs-sys' } | Select-Object -First 1
$QuickJsNote = if ($QuickJsPackage) {
  "经 rquickjs-sys $($QuickJsPackage.version) 捆绑"
} else {
  '经 rquickjs 捆绑，Cargo.lock 未记录具体版本'
}

# ---------------------------------------------------------------------------
# Render markdown
# ---------------------------------------------------------------------------

$Builder = New-Object System.Text.StringBuilder
function Add-Line {
  param([string]$Line = '')
  [void]$Builder.Append($Line).Append("`n")
}

Add-Line '# 第三方许可清单'
Add-Line ''
Add-Line ("产品：Veil {0}" -f $WorkspaceVersion)
Add-Line '生成命令：`pwsh -NoProfile -File tools/third_party_notices.ps1`'
Add-Line '校验命令：`pwsh -NoProfile -File tools/third_party_notices.ps1 -Check`'
Add-Line ''
Add-Line '本文件由脚本根据 `cargo metadata --locked --format-version 1` 与仓库内嵌资产清单自动生成，请勿手改。发布门禁以 `-Check` 校验其是否与当前依赖一致。'
Add-Line ''
Add-Line ("- Rust 包（含 workspace 成员）：{0}" -f $Packages.Count)
Add-Line '- 内嵌第三方资产：3（Mermaid、linkedom、QuickJS）'
Add-Line ("- 无本地许可证全文的包：{0}" -f $NoText.Count)
Add-Line ("- 许可证全文条目（按内容 SHA-256 去重）：{0}" -f $Sections.Count)
Add-Line ''
Add-Line '## Rust 依赖组件'
Add-Line ''
Add-Line '| Package | Version | License | Repository |'
Add-Line '| --- | --- | --- | --- |'
foreach ($package in $Packages) {
  $isWorkspaceMember = $WorkspaceIds -contains $package.id
  $license = if ([string]::IsNullOrWhiteSpace($package.license)) {
    if ($isWorkspaceMember) { 'MIT (workspace)' } else { 'UNSPECIFIED' }
  } else {
    $package.license
  }
  Add-Line ('| {0} | {1} | {2} | {3} |' -f (Format-Cell $package.name), (Format-Cell $package.version), (Format-Cell $license), (Format-Cell $package.repository))
}
Add-Line ''

Add-Line '## 内嵌第三方资产'
Add-Line ''
Add-Line '以下资产随 `veil.exe` 分发，但不通过 Cargo 依赖引入。'
Add-Line ''
Add-Line ("### Mermaid {0}" -f $MermaidVersion)
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line '- 来源：`crates/veil-mermaid-offline/assets/mermaid.min.js`'
Add-Line ("- 版本来源：{0}" -f $MermaidVersionNote)
Add-Line '- 版权：归上游 Mermaid 项目及其贡献者所有；本仓库未附带独立的版权声明文件，本清单不代填版权人名称。'
Add-Line ''
Add-Line '### linkedom（bundled）'
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line '- 来源：`crates/veil-mermaid-offline/assets/linkedom.worker.mjs`（构建时由 `build.rs` 转换为经典脚本）'
Add-Line '- 版本：bundled（打包产物未记录上游版本号；以上游项目发布记录为准）'
Add-Line '- 版权：归上游 linkedom 项目及其贡献者所有；本仓库未附带独立的版权声明文件，本清单不代填版权人名称。'
Add-Line ''
Add-Line '### QuickJS（bundled）'
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line ('- 来源：' + $QuickJsNote + ' 的 QuickJS 源码，经 Rust 依赖 `rquickjs` 编译进 `veil.exe`')
Add-Line '- 版本：bundled（由 rquickjs-sys 随 crate 固定，具体版本见 Cargo.lock）'
Add-Line '- 版权：归 QuickJS 上游作者及其贡献者所有；本仓库未附带独立的版权声明文件，本清单不代填版权人名称。'
Add-Line ''
Add-Line '### MIT 声明'
Add-Line ''
Add-Line '以上内嵌资产均以 MIT 许可证授权。具体版权行由各上游项目在其发布物中声明，本仓库未包含这些版权行，故此处不代填；下列条款为 MIT 许可证正文，版权行以各上游项目为准。'
Add-Line ''
Add-Line '~~~text'
Add-Line 'MIT License'
Add-Line ''
Add-Line 'Copyright (c) <各上游项目的版权持有人>'
Add-Line ''
Add-Line 'Permission is hereby granted, free of charge, to any person obtaining a copy'
Add-Line 'of this software and associated documentation files (the "Software"), to deal'
Add-Line 'in the Software without restriction, including without limitation the rights'
Add-Line 'to use, copy, modify, merge, publish, distribute, sublicense, and/or sell'
Add-Line 'copies of the Software, and to permit persons to whom the Software is'
Add-Line 'furnished to do so, subject to the following conditions:'
Add-Line ''
Add-Line 'The above copyright notice and this permission notice shall be included in all'
Add-Line 'copies or substantial portions of the Software.'
Add-Line ''
Add-Line 'THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR'
Add-Line 'IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,'
Add-Line 'FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE'
Add-Line 'AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER'
Add-Line 'LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,'
Add-Line 'OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE'
Add-Line 'SOFTWARE.'
Add-Line '~~~'
Add-Line ''

Add-Line '## 许可证全文'
Add-Line ''
Add-Line '以下文本来自各包 manifest 同目录的 `LICENSE*`、`COPYING*`、`NOTICE*` 文件；按内容 SHA-256 去重并按哈希值排序，每份文本前列出适用包与来源文件名。'
Add-Line ''
$hashes = @($Sections.Keys)
[Array]::Sort($hashes, [System.StringComparer]::Ordinal)
foreach ($hash in $hashes) {
  $section = $Sections[$hash]
  Add-Line ("### {0}" -f $hash)
  Add-Line ''
  Add-Line '适用包：'
  foreach ($entry in $section.Entries) { Add-Line ("- {0}" -f $entry) }
  Add-Line ''
  $fence = '~~~'
  if ($section.Content.Contains('~~~')) { $fence = '````' }
  Add-Line ("{0}text" -f $fence)
  [void]$Builder.Append($section.Content)
  Add-Line $fence
  Add-Line ''
}

Add-Line '## 无本地许可证全文的包'
Add-Line ''
Add-Line '以下包仅在 `Cargo.toml` 中声明 SPDX 许可证表达式，其 crate 源目录未附带 `LICENSE*`、`COPYING*`、`NOTICE*` 文件。完整条款请查阅 Cargo 源目录（通常为 `$CARGO_HOME/registry/src/<registry>/<name>-<version>/`）或 crates.io 页面。'
Add-Line ''
if ($NoText.Count -eq 0) {
  Add-Line '- 无'
} else {
  foreach ($entry in $NoText) {
    Add-Line ('- `{0}` {1} — SPDX：`{2}` — 源目录：`$CARGO_HOME/registry/src/<registry>/{0}-{1}/`' -f $entry.Name, $entry.Version, $entry.License)
  }
}
Add-Line ''

$GeneratedText = $Builder.ToString()
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

# ---------------------------------------------------------------------------
# Write or check
# ---------------------------------------------------------------------------

if ($Check) {
  if (-not (Test-Path -LiteralPath $OutFile)) {
    Write-Host "CHECK FAIL: $OutFile does not exist; run tools/third_party_notices.ps1 to generate it" -ForegroundColor Red
    exit 1
  }

  $expectedBytes = $Utf8NoBom.GetBytes($GeneratedText)
  $actualBytes = [System.IO.File]::ReadAllBytes($OutFile)
  $expectedHash = Get-TextHash -Text $GeneratedText
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $actualHash = ([System.BitConverter]::ToString($sha.ComputeHash($actualBytes))).Replace('-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }

  if ($actualHash -eq $expectedHash -and $actualBytes.Length -eq $expectedBytes.Length) {
    Write-Host "OK: THIRD-PARTY-LICENSES.md is up to date ($($Packages.Count) packages, $($NoText.Count) without local full text)" -ForegroundColor Green
    exit 0
  }

  $actualText = [System.IO.File]::ReadAllText($OutFile, [System.Text.Encoding]::UTF8)
  $actualLines = $actualText -split "`r?`n"
  $expectedLines = $GeneratedText -split "`n"
  $maxLines = [Math]::Max($actualLines.Count, $expectedLines.Count)
  $diffCount = 0
  $firstDiffLine = -1
  $samples = New-Object System.Collections.Generic.List[string]
  for ($i = 0; $i -lt $maxLines; $i++) {
    $actualLine = if ($i -lt $actualLines.Count) { $actualLines[$i] } else { '<missing>' }
    $expectedLine = if ($i -lt $expectedLines.Count) { $expectedLines[$i] } else { '<missing>' }
    if (-not [string]::Equals($actualLine, $expectedLine, [System.StringComparison]::Ordinal)) {
      $diffCount++
      if ($firstDiffLine -lt 0) { $firstDiffLine = $i + 1 }
      if ($samples.Count -lt 5) {
        $samples.Add(("  line {0}`n    on disk:   {1}`n    generated: {2}" -f ($i + 1), $actualLine, $expectedLine))
      }
    }
  }

  Write-Host "CHECK FAIL: THIRD-PARTY-LICENSES.md is stale" -ForegroundColor Red
  Write-Host ("  on disk:   {0} bytes, SHA-256 {1}" -f $actualBytes.Length, $actualHash)
  Write-Host ("  generated: {0} bytes, SHA-256 {1}" -f $expectedBytes.Length, $expectedHash)
  Write-Host ("  {0} differing line(s); first at line {1}" -f $diffCount, $firstDiffLine)
  foreach ($sample in $samples) { Write-Host $sample }
  Write-Host '  Run: pwsh -NoProfile -File tools/third_party_notices.ps1'
  exit 1
}

$parent = Split-Path -Parent $OutFile
if (-not [string]::IsNullOrWhiteSpace($parent)) {
  New-Item -ItemType Directory -Force -Path $parent | Out-Null
}
[System.IO.File]::WriteAllText($OutFile, $GeneratedText, $Utf8NoBom)
Write-Host "Wrote $OutFile" -ForegroundColor Green
Write-Host ("  packages: {0}; without local full text: {1}; license text sections: {2}" -f $Packages.Count, $NoText.Count, $Sections.Count)
exit 0
