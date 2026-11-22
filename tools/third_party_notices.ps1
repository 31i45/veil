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

# License-ish file names. The suffix branch accepts LICENSE-MIT, COPYING.LIB, NOTICE.md, ...
$LicenseNamePattern = '^(?i)(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)([-._][A-Za-z0-9._-]+)?$'
# Source files such as `src/copying.rs` must not be mistaken for `COPYING*` license files.
$SourceFilePattern = '\.(?i)(rs|c|h|cc|cpp|cxx|hpp|hh|js|mjs|cjs|ts|tsx|py|go|java|kt|rb|php|cs|swift|scala|sh|ps1|bat|cmd|toml|json|yml|yaml|lock|in|cfg)$'

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

function Get-NormalizedString {
  param([string]$Text)
  if ($null -eq $Text) { return '' }
  $value = $Text.TrimStart([char]0xFEFF)
  $value = $value -replace "`r`n", "`n" -replace "`r", "`n"
  return $value.TrimEnd() + "`n"
}

function Get-NormalizedText {
  param([string]$Path)
  return Get-NormalizedString -Text ([System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8))
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

function Test-LicenseFileName {
  param([string]$Name)
  if ([string]::IsNullOrWhiteSpace($Name)) { return $false }
  return ($Name -match $LicenseNamePattern) -and ($Name -notmatch $SourceFilePattern)
}

function Get-RelativeLabel {
  param([string]$PackageDir, [string]$FullPath)

  $rootFull = [System.IO.Path]::GetFullPath($PackageDir).TrimEnd('\', '/')
  $fileFull = [System.IO.Path]::GetFullPath($FullPath)
  if ($fileFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    return $fileFull.Substring($rootFull.Length).TrimStart('\', '/').Replace('\', '/')
  }
  return [System.IO.Path]::GetFileName($FullPath)
}

function Sort-FileEntries {
  param($Entries)

  $array = @($Entries)
  $comparison = [System.Comparison[object]] {
    param($Left, $Right)
    $depthResult = [int]$Left.Depth - [int]$Right.Depth
    if ($depthResult -ne 0) { return $depthResult }
    return [string]::CompareOrdinal([string]$Left.Label, [string]$Right.Label)
  }
  [Array]::Sort($array, $comparison)
  return $array
}

function Find-LicenseFiles {
  param($Package, [bool]$IsWorkspaceMember)

  $dir = Split-Path -Parent $Package.manifest_path

  # 1. manifest `license_file`, when declared, is authoritative.
  if (-not [string]::IsNullOrWhiteSpace($Package.license_file)) {
    $declared = if ([System.IO.Path]::IsPathRooted($Package.license_file)) {
      $Package.license_file
    } else {
      Join-Path $dir $Package.license_file
    }
    if (Test-Path -LiteralPath $declared -PathType Leaf) {
      $full = (Resolve-Path -LiteralPath $declared).Path
      return [pscustomobject]@{
        Kind = 'declared'
        Files = @([pscustomobject]@{ Path = $full; Label = (Get-RelativeLabel -PackageDir $dir -FullPath $full); Depth = 0 })
      }
    }
  }

  if (-not (Test-Path -LiteralPath $dir)) {
    return [pscustomobject]@{ Kind = 'none'; Files = @() }
  }

  # 2. Package root wins over subdirectories.
  $rootFiles = @(Get-ChildItem -LiteralPath $dir -File -ErrorAction SilentlyContinue |
    Where-Object { Test-LicenseFileName -Name $_.Name })
  if ($rootFiles.Count -gt 0) {
    $comparison = [System.Comparison[object]] {
      param($Left, $Right)
      return [string]::CompareOrdinal([string]$Left.Name, [string]$Right.Name)
    }
    [Array]::Sort($rootFiles, $comparison)
    return [pscustomobject]@{
      Kind = 'root'
      Files = @($rootFiles | ForEach-Object {
        [pscustomobject]@{ Path = $_.FullName; Label = $_.Name; Depth = 0 }
      })
    }
  }

  # 3. Breadth-first search of subdirectories, at most three levels deep, skipping `target/`.
  $found = [System.Collections.Generic.List[object]]::new()
  $queue = [System.Collections.Generic.Queue[object]]::new()
  $queue.Enqueue([pscustomobject]@{ Path = $dir; Depth = 0 })
  while ($queue.Count -gt 0) {
    $current = $queue.Dequeue()
    if ($current.Depth -ge 3) { continue }
    $childDirs = @(Get-ChildItem -LiteralPath $current.Path -Directory -Force -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -ne 'target' } |
      Sort-Object -Property Name)
    foreach ($child in $childDirs) {
      $depth = $current.Depth + 1
      $files = @(Get-ChildItem -LiteralPath $child.FullName -File -ErrorAction SilentlyContinue |
        Where-Object { Test-LicenseFileName -Name $_.Name })
      foreach ($file in $files) {
        $found.Add([pscustomobject]@{
          Path = $file.FullName
          Label = (Get-RelativeLabel -PackageDir $dir -FullPath $file.FullName)
          Depth = $depth
        })
      }
      $queue.Enqueue([pscustomobject]@{ Path = $child.FullName; Depth = $depth })
    }
  }

  if ($found.Count -gt 0) {
    return [pscustomobject]@{ Kind = 'recursive'; Files = @(Sort-FileEntries -Entries $found) }
  }

  # 4. Workspace members fall back to the repository root LICENSE.
  if ($IsWorkspaceMember -and (Test-Path -LiteralPath $RootLicense -PathType Leaf)) {
    return [pscustomobject]@{
      Kind = 'workspace'
      Files = @([pscustomobject]@{ Path = $RootLicense; Label = 'LICENSE (workspace root)'; Depth = 0 })
    }
  }

  return [pscustomobject]@{ Kind = 'none'; Files = @() }
}

# Permissive licenses selected for SPDX OR expressions, most preferred first.
$PermissiveLicenseOrder = @(
  'MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib',
  'BSL-1.0', 'CC0-1.0', '0BSD', 'Unlicense', 'Unicode-3.0'
)

function Get-LicensePriority {
  param([string]$Id)

  $index = [Array]::IndexOf($PermissiveLicenseOrder, $Id)
  if ($index -ge 0) { return $index }
  return 1000
}

function Get-SelectedLicenseIds {
  param([string]$Expression)

  if ([string]::IsNullOrWhiteSpace($Expression)) { return @() }

  # `/` is the legacy Rust syntax for a choice between licenses; parentheses only group terms.
  $text = $Expression -replace '/', ' OR ' -replace '\(', ' ' -replace '\)', ' '
  $alternatives = [System.Collections.Generic.List[object]]::new()
  foreach ($part in [regex]::Split($text, '(?i)\s+OR\s+')) {
    $ids = [System.Collections.Generic.List[string]]::new()
    foreach ($piece in [regex]::Split($part, '(?i)\s+AND\s+')) {
      $token = $piece.Trim()
      if ([string]::IsNullOrWhiteSpace($token)) { continue }
      if ($token -match '^(?i)WITH$') { continue }
      if (-not $ids.Contains($token)) { $ids.Add($token) }
    }
    if ($ids.Count -gt 0) { $alternatives.Add($ids) }
  }

  if ($alternatives.Count -eq 0) { return @() }

  # Pick the most permissive alternative; non-whitelisted licenses score 1000 and are used
  # only when the expression offers nothing from the whitelist.
  $best = $null
  $bestScore = [int]::MaxValue
  foreach ($ids in $alternatives) {
    $score = 1000
    foreach ($id in $ids) {
      $priority = Get-LicensePriority -Id $id
      if ($priority -lt $score) { $score = $priority }
    }
    if ($score -lt $bestScore) {
      $bestScore = $score
      $best = $ids
    }
  }

  return @($best)
}

function Format-Authors {
  param($Authors)

  $items = @($Authors | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
  if ($items.Count -eq 0) { return '未声明' }

  # Render `Name <mail>` as `Name (mail)` so this file contains no angle-bracket placeholders.
  $rendered = @($items | ForEach-Object { ($_ -replace '<([^>]+)>', '($1)').Trim() })
  return ($rendered -join '；')
}

function Convert-AngleDelimiters {
  param([string]$Text)

  # Upstream license text sometimes wraps mails, URLs or template fields in `<...>`.
  # Render those delimiters as parentheses so this file contains no angle-bracket placeholders.
  $value = $Text
  while ($value -match '<[^<>]*>') {
    $value = [regex]::Replace($value, '<([^<>]*)>', '($1)')
  }
  return $value
}

function Get-FencedBlock {
  param([string]$Text)
  $safe = Convert-AngleDelimiters -Text $Text
  $fence = '~~~'
  if ($safe.Contains('~~~')) { $fence = '````' }
  return ("{0}text`n{1}{0}`n" -f $fence, $safe)
}

function Get-StandardLicenseTexts {
  $texts = [ordered]@{}

  $texts['MIT'] = @'
MIT License

版权归属：见各包元数据 authors 与 repository。

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
'@

  $texts['Apache-2.0'] = @'
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS
'@

  $texts['Zlib'] = @'
zlib License

This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
'@

  $texts['BSL-1.0'] = @'
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
'@

  $texts['LGPL-2.1-or-later'] = @'

                  GNU LESSER GENERAL PUBLIC LICENSE
                       Version 2.1, February 1999

 Copyright (C) 1991, 1999 Free Software Foundation, Inc.
	51 Franklin St, Fifth Floor, Boston, MA  02110-1301  USA
 Everyone is permitted to copy and distribute verbatim copies
 of this license document, but changing it is not allowed.

[This is the first released version of the Lesser GPL.  It also counts
 as the successor of the GNU Library Public License, version 2, hence
 the version number 2.1.]

                            Preamble

  The licenses for most software are designed to take away your
freedom to share and change it.  By contrast, the GNU General Public
Licenses are intended to guarantee your freedom to share and change
free software--to make sure the software is free for all its users.

  This license, the Lesser General Public License, applies to some
specially designated software packages--typically libraries--of the
Free Software Foundation and other authors who decide to use it.  You
can use it too, but we suggest you first think carefully about whether
this license or the ordinary General Public License is the better
strategy to use in any particular case, based on the explanations
below.

  When we speak of free software, we are referring to freedom of use,
not price.  Our General Public Licenses are designed to make sure that
you have the freedom to distribute copies of free software (and charge
for this service if you wish); that you receive source code or can get
it if you want it; that you can change the software and use pieces of
it in new free programs; and that you are informed that you can do
these things.

  To protect your rights, we need to make restrictions that forbid
distributors to deny you these rights or to ask you to surrender these
rights.  These restrictions translate to certain responsibilities for
you if you distribute copies of the library or if you modify it.

  For example, if you distribute copies of the library, whether gratis
or for a fee, you must give the recipients all the rights that we gave
you.  You must make sure that they, too, receive or can get the source
code.  If you link other code with the library, you must provide
complete object files to the recipients, so that they can relink them
with the library after making changes to the library and recompiling
it.  And you must show them these terms so they know their rights.

  We protect your rights with a two-step method: (1) we copyright the
library, and (2) we offer you this license, which gives you legal
permission to copy, distribute and/or modify the library.

  To protect each distributor, we want to make it very clear that
there is no warranty for the free library.  Also, if the library is
modified by someone else and passed on, the recipients should know
that what they have is not the original version, so that the original
author's reputation will not be affected by problems that might be
introduced by others.

  Finally, software patents pose a constant threat to the existence of
any free program.  We wish to make sure that a company cannot
effectively restrict the users of a free program by obtaining a
restrictive license from a patent holder.  Therefore, we insist that
any patent license obtained for a version of the library must be
consistent with the full freedom of use specified in this license.

  Most GNU software, including some libraries, is covered by the
ordinary GNU General Public License.  This license, the GNU Lesser
General Public License, applies to certain designated libraries, and
is quite different from the ordinary General Public License.  We use
this license for certain libraries in order to permit linking those
libraries into non-free programs.

  When a program is linked with a library, whether statically or using
a shared library, the combination of the two is legally speaking a
combined work, a derivative of the original library.  The ordinary
General Public License therefore permits such linking only if the
entire combination fits its criteria of freedom.  The Lesser General
Public License permits more lax criteria for linking other code with
the library.

  We call this license the "Lesser" General Public License because it
does Less to protect the user's freedom than the ordinary General
Public License.  It also provides other free software developers Less
of an advantage over competing non-free programs.  These disadvantages
are the reason we use the ordinary General Public License for many
libraries.  However, the Lesser license provides advantages in certain
special circumstances.

  For example, on rare occasions, there may be a special need to
encourage the widest possible use of a certain library, so that it
becomes a de-facto standard.  To achieve this, non-free programs must
be allowed to use the library.  A more frequent case is that a free
library does the same job as widely used non-free libraries.  In this
case, there is little to gain by limiting the free library to free
software only, so we use the Lesser General Public License.

  In other cases, permission to use a particular library in non-free
programs enables a greater number of people to use a large body of
free software.  For example, permission to use the GNU C Library in
non-free programs enables many more people to use the whole GNU
operating system, as well as its variant, the GNU/Linux operating
system.

  Although the Lesser General Public License is Less protective of the
users' freedom, it does ensure that the user of a program that is
linked with the Library has the freedom and the wherewithal to run
that program using a modified version of the Library.

  The precise terms and conditions for copying, distribution and
modification follow.  Pay close attention to the difference between a
"work based on the library" and a "work that uses the library".  The
former contains code derived from the library, whereas the latter must
be combined with the library in order to run.

                  GNU LESSER GENERAL PUBLIC LICENSE
   TERMS AND CONDITIONS FOR COPYING, DISTRIBUTION AND MODIFICATION

  0. This License Agreement applies to any software library or other
program which contains a notice placed by the copyright holder or
other authorized party saying it may be distributed under the terms of
this Lesser General Public License (also called "this License").
Each licensee is addressed as "you".

  A "library" means a collection of software functions and/or data
prepared so as to be conveniently linked with application programs
(which use some of those functions and data) to form executables.

  The "Library", below, refers to any such software library or work
which has been distributed under these terms.  A "work based on the
Library" means either the Library or any derivative work under
copyright law: that is to say, a work containing the Library or a
portion of it, either verbatim or with modifications and/or translated
straightforwardly into another language.  (Hereinafter, translation is
included without limitation in the term "modification".)

  "Source code" for a work means the preferred form of the work for
making modifications to it.  For a library, complete source code means
all the source code for all modules it contains, plus any associated
interface definition files, plus the scripts used to control
compilation and installation of the library.

  Activities other than copying, distribution and modification are not
covered by this License; they are outside its scope.  The act of
running a program using the Library is not restricted, and output from
such a program is covered only if its contents constitute a work based
on the Library (independent of the use of the Library in a tool for
writing it).  Whether that is true depends on what the Library does
and what the program that uses the Library does.

  1. You may copy and distribute verbatim copies of the Library's
complete source code as you receive it, in any medium, provided that
you conspicuously and appropriately publish on each copy an
appropriate copyright notice and disclaimer of warranty; keep intact
all the notices that refer to this License and to the absence of any
warranty; and distribute a copy of this License along with the
Library.

  You may charge a fee for the physical act of transferring a copy,
and you may at your option offer warranty protection in exchange for a
fee.

  2. You may modify your copy or copies of the Library or any portion
of it, thus forming a work based on the Library, and copy and
distribute such modifications or work under the terms of Section 1
above, provided that you also meet all of these conditions:

    a) The modified work must itself be a software library.

    b) You must cause the files modified to carry prominent notices
    stating that you changed the files and the date of any change.

    c) You must cause the whole of the work to be licensed at no
    charge to all third parties under the terms of this License.

    d) If a facility in the modified Library refers to a function or a
    table of data to be supplied by an application program that uses
    the facility, other than as an argument passed when the facility
    is invoked, then you must make a good faith effort to ensure that,
    in the event an application does not supply such function or
    table, the facility still operates, and performs whatever part of
    its purpose remains meaningful.

    (For example, a function in a library to compute square roots has
    a purpose that is entirely well-defined independent of the
    application.  Therefore, Subsection 2d requires that any
    application-supplied function or table used by this function must
    be optional: if the application does not supply it, the square
    root function must still compute square roots.)

These requirements apply to the modified work as a whole.  If
identifiable sections of that work are not derived from the Library,
and can be reasonably considered independent and separate works in
themselves, then this License, and its terms, do not apply to those
sections when you distribute them as separate works.  But when you
distribute the same sections as part of a whole which is a work based
on the Library, the distribution of the whole must be on the terms of
this License, whose permissions for other licensees extend to the
entire whole, and thus to each and every part regardless of who wrote
it.

Thus, it is not the intent of this section to claim rights or contest
your rights to work written entirely by you; rather, the intent is to
exercise the right to control the distribution of derivative or
collective works based on the Library.

In addition, mere aggregation of another work not based on the Library
with the Library (or with a work based on the Library) on a volume of
a storage or distribution medium does not bring the other work under
the scope of this License.

  3. You may opt to apply the terms of the ordinary GNU General Public
License instead of this License to a given copy of the Library.  To do
this, you must alter all the notices that refer to this License, so
that they refer to the ordinary GNU General Public License, version 2,
instead of to this License.  (If a newer version than version 2 of the
ordinary GNU General Public License has appeared, then you can specify
that version instead if you wish.)  Do not make any other change in
these notices.

  Once this change is made in a given copy, it is irreversible for
that copy, so the ordinary GNU General Public License applies to all
subsequent copies and derivative works made from that copy.

  This option is useful when you wish to copy part of the code of
the Library into a program that is not a library.

  4. You may copy and distribute the Library (or a portion or
derivative of it, under Section 2) in object code or executable form
under the terms of Sections 1 and 2 above provided that you accompany
it with the complete corresponding machine-readable source code, which
must be distributed under the terms of Sections 1 and 2 above on a
medium customarily used for software interchange.

  If distribution of object code is made by offering access to copy
from a designated place, then offering equivalent access to copy the
source code from the same place satisfies the requirement to
distribute the source code, even though third parties are not
compelled to copy the source along with the object code.

  5. A program that contains no derivative of any portion of the
Library, but is designed to work with the Library by being compiled or
linked with it, is called a "work that uses the Library".  Such a
work, in isolation, is not a derivative work of the Library, and
therefore falls outside the scope of this License.

  However, linking a "work that uses the Library" with the Library
creates an executable that is a derivative of the Library (because it
contains portions of the Library), rather than a "work that uses the
library".  The executable is therefore covered by this License.
Section 6 states terms for distribution of such executables.

  When a "work that uses the Library" uses material from a header file
that is part of the Library, the object code for the work may be a
derivative work of the Library even though the source code is not.
Whether this is true is especially significant if the work can be
linked without the Library, or if the work is itself a library.  The
threshold for this to be true is not precisely defined by law.

  If such an object file uses only numerical parameters, data
structure layouts and accessors, and small macros and small inline
functions (ten lines or less in length), then the use of the object
file is unrestricted, regardless of whether it is legally a derivative
work.  (Executables containing this object code plus portions of the
Library will still fall under Section 6.)

  Otherwise, if the work is a derivative of the Library, you may
distribute the object code for the work under the terms of Section 6.
Any executables containing that work also fall under Section 6,
whether or not they are linked directly with the Library itself.

  6. As an exception to the Sections above, you may also combine or
link a "work that uses the Library" with the Library to produce a
work containing portions of the Library, and distribute that work
under terms of your choice, provided that the terms permit
modification of the work for the customer's own use and reverse
engineering for debugging such modifications.

  You must give prominent notice with each copy of the work that the
Library is used in it and that the Library and its use are covered by
this License.  You must supply a copy of this License.  If the work
during execution displays copyright notices, you must include the
copyright notice for the Library among them, as well as a reference
directing the user to the copy of this License.  Also, you must do one
of these things:

    a) Accompany the work with the complete corresponding
    machine-readable source code for the Library including whatever
    changes were used in the work (which must be distributed under
    Sections 1 and 2 above); and, if the work is an executable linked
    with the Library, with the complete machine-readable "work that
    uses the Library", as object code and/or source code, so that the
    user can modify the Library and then relink to produce a modified
    executable containing the modified Library.  (It is understood
    that the user who changes the contents of definitions files in the
    Library will not necessarily be able to recompile the application
    to use the modified definitions.)

    b) Use a suitable shared library mechanism for linking with the
    Library.  A suitable mechanism is one that (1) uses at run time a
    copy of the library already present on the user's computer system,
    rather than copying library functions into the executable, and (2)
    will operate properly with a modified version of the library, if
    the user installs one, as long as the modified version is
    interface-compatible with the version that the work was made with.

    c) Accompany the work with a written offer, valid for at least
    three years, to give the same user the materials specified in
    Subsection 6a, above, for a charge no more than the cost of
    performing this distribution.

    d) If distribution of the work is made by offering access to copy
    from a designated place, offer equivalent access to copy the above
    specified materials from the same place.

    e) Verify that the user has already received a copy of these
    materials or that you have already sent this user a copy.

  For an executable, the required form of the "work that uses the
Library" must include any data and utility programs needed for
reproducing the executable from it.  However, as a special exception,
the materials to be distributed need not include anything that is
normally distributed (in either source or binary form) with the major
components (compiler, kernel, and so on) of the operating system on
which the executable runs, unless that component itself accompanies
the executable.

  It may happen that this requirement contradicts the license
restrictions of other proprietary libraries that do not normally
accompany the operating system.  Such a contradiction means you cannot
use both them and the Library together in an executable that you
distribute.

  7. You may place library facilities that are a work based on the
Library side-by-side in a single library together with other library
facilities not covered by this License, and distribute such a combined
library, provided that the separate distribution of the work based on
the Library and of the other library facilities is otherwise
permitted, and provided that you do these two things:

    a) Accompany the combined library with a copy of the same work
    based on the Library, uncombined with any other library
    facilities.  This must be distributed under the terms of the
    Sections above.

    b) Give prominent notice with the combined library of the fact
    that part of it is a work based on the Library, and explaining
    where to find the accompanying uncombined form of the same work.

  8. You may not copy, modify, sublicense, link with, or distribute
the Library except as expressly provided under this License.  Any
attempt otherwise to copy, modify, sublicense, link with, or
distribute the Library is void, and will automatically terminate your
rights under this License.  However, parties who have received copies,
or rights, from you under this License will not have their licenses
terminated so long as such parties remain in full compliance.

  9. You are not required to accept this License, since you have not
signed it.  However, nothing else grants you permission to modify or
distribute the Library or its derivative works.  These actions are
prohibited by law if you do not accept this License.  Therefore, by
modifying or distributing the Library (or any work based on the
Library), you indicate your acceptance of this License to do so, and
all its terms and conditions for copying, distributing or modifying
the Library or works based on it.

  10. Each time you redistribute the Library (or any work based on the
Library), the recipient automatically receives a license from the
original licensor to copy, distribute, link with or modify the Library
subject to these terms and conditions.  You may not impose any further
restrictions on the recipients' exercise of the rights granted herein.
You are not responsible for enforcing compliance by third parties with
this License.

  11. If, as a consequence of a court judgment or allegation of patent
infringement or for any other reason (not limited to patent issues),
conditions are imposed on you (whether by court order, agreement or
otherwise) that contradict the conditions of this License, they do not
excuse you from the conditions of this License.  If you cannot
distribute so as to satisfy simultaneously your obligations under this
License and any other pertinent obligations, then as a consequence you
may not distribute the Library at all.  For example, if a patent
license would not permit royalty-free redistribution of the Library by
all those who receive copies directly or indirectly through you, then
the only way you could satisfy both it and this License would be to
refrain entirely from distribution of the Library.

If any portion of this section is held invalid or unenforceable under
any particular circumstance, the balance of the section is intended to
apply, and the section as a whole is intended to apply in other
circumstances.

It is not the purpose of this section to induce you to infringe any
patents or other property right claims or to contest validity of any
such claims; this section has the sole purpose of protecting the
integrity of the free software distribution system which is
implemented by public license practices.  Many people have made
generous contributions to the wide range of software distributed
through that system in reliance on consistent application of that
system; it is up to the author/donor to decide if he or she is willing
to distribute software through any other system and a licensee cannot
impose that choice.

This section is intended to make thoroughly clear what is believed to
be a consequence of the rest of this License.

  12. If the distribution and/or use of the Library is restricted in
certain countries either by patents or by copyrighted interfaces, the
original copyright holder who places the Library under this License
may add an explicit geographical distribution limitation excluding those
countries, so that distribution is permitted only in or among
countries not thus excluded.  In such case, this License incorporates
the limitation as if written in the body of this License.

  13. The Free Software Foundation may publish revised and/or new
versions of the Lesser General Public License from time to time.
Such new versions will be similar in spirit to the present version,
but may differ in detail to address new problems or concerns.

Each version is given a distinguishing version number.  If the Library
specifies a version number of this License which applies to it and
"any later version", you have the option of following the terms and
conditions either of that version or of any later version published by
the Free Software Foundation.  If the Library does not specify a
license version number, you may choose any version ever published by
the Free Software Foundation.

  14. If you wish to incorporate parts of the Library into other free
programs whose distribution conditions are incompatible with these,
write to the author to ask for permission.  For software which is
copyrighted by the Free Software Foundation, write to the Free
Software Foundation; we sometimes make exceptions for this.  Our
decision will be guided by the two goals of preserving the free status
of all derivatives of our free software and of promoting the sharing
and reuse of software generally.

                            NO WARRANTY

  15. BECAUSE THE LIBRARY IS LICENSED FREE OF CHARGE, THERE IS NO
WARRANTY FOR THE LIBRARY, TO THE EXTENT PERMITTED BY APPLICABLE LAW.
EXCEPT WHEN OTHERWISE STATED IN WRITING THE COPYRIGHT HOLDERS AND/OR
OTHER PARTIES PROVIDE THE LIBRARY "AS IS" WITHOUT WARRANTY OF ANY
KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE.  THE ENTIRE RISK AS TO THE QUALITY AND PERFORMANCE OF THE
LIBRARY IS WITH YOU.  SHOULD THE LIBRARY PROVE DEFECTIVE, YOU ASSUME
THE COST OF ALL NECESSARY SERVICING, REPAIR OR CORRECTION.

  16. IN NO EVENT UNLESS REQUIRED BY APPLICABLE LAW OR AGREED TO IN
WRITING WILL ANY COPYRIGHT HOLDER, OR ANY OTHER PARTY WHO MAY MODIFY
AND/OR REDISTRIBUTE THE LIBRARY AS PERMITTED ABOVE, BE LIABLE TO YOU
FOR DAMAGES, INCLUDING ANY GENERAL, SPECIAL, INCIDENTAL OR
CONSEQUENTIAL DAMAGES ARISING OUT OF THE USE OR INABILITY TO USE THE
LIBRARY (INCLUDING BUT NOT LIMITED TO LOSS OF DATA OR DATA BEING
RENDERED INACCURATE OR LOSSES SUSTAINED BY YOU OR THIRD PARTIES OR A
FAILURE OF THE LIBRARY TO OPERATE WITH ANY OTHER SOFTWARE), EVEN IF
SUCH HOLDER OR OTHER PARTY HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
DAMAGES.

                     END OF TERMS AND CONDITIONS
'@

  return $texts
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

$rawLicenseTexts = Get-StandardLicenseTexts
$StandardLicenseTexts = @{}
foreach ($key in $rawLicenseTexts.Keys) {
  $StandardLicenseTexts[$key] = Get-NormalizedString -Text $rawLicenseTexts[$key]
}

$Sections = @{}
$SourceCounts = [ordered]@{ root = 0; recursive = 0; declared = 0; workspace = 0; none = 0 }
$NoText = [System.Collections.Generic.List[object]]::new()

foreach ($package in $Packages) {
  $isWorkspaceMember = $WorkspaceIds -contains $package.id
  $found = Find-LicenseFiles -Package $package -IsWorkspaceMember $isWorkspaceMember
  $SourceCounts[$found.Kind] = $SourceCounts[$found.Kind] + 1

  $licenseExpression = if ([string]::IsNullOrWhiteSpace($package.license)) {
    if ($isWorkspaceMember) { 'MIT (workspace)' } else { '' }
  } else {
    $package.license
  }

  if ($found.Files.Count -eq 0) {
    $NoText.Add([pscustomobject]@{
      Name = $package.name
      Version = $package.version
      License = $licenseExpression
      Repository = $package.repository
      Authors = $package.authors
      Ids = @(Get-SelectedLicenseIds -Expression $licenseExpression)
    })
    continue
  }

  foreach ($entry in $found.Files) {
    $content = Get-NormalizedText -Path $entry.Path
    $hash = Get-TextHash -Text $content
    if (-not $Sections.ContainsKey($hash)) {
      $Sections[$hash] = [pscustomobject]@{
        Content = $content
        Entries = [System.Collections.Generic.List[string]]::new()
      }
    }
    $Sections[$hash].Entries.Add(("``{0}`` {1} (``{2}``)" -f $package.name, $package.version, $entry.Label))
  }
}

# Group packages without local license files by every SPDX id in their expression.
$GapGroups = [System.Collections.Generic.SortedDictionary[string, object]]::new([System.StringComparer]::Ordinal)
$Unspecified = [System.Collections.Generic.List[object]]::new()
foreach ($entry in $NoText) {
  if ($entry.Ids.Count -eq 0) {
    $Unspecified.Add($entry)
    continue
  }
  foreach ($id in $entry.Ids) {
    if (-not $GapGroups.ContainsKey($id)) {
      $GapGroups[$id] = [System.Collections.Generic.List[object]]::new()
    }
    $GapGroups[$id].Add($entry)
  }
}

# Embedded asset metadata (versions are read from the packaged files, never guessed).
$MermaidVersion = 'bundled'
$MermaidVersionNote = '打包文件未记录版本号'
$mermaidPath = Join-Path $AssetDir 'mermaid.min.js'
$MermaidBanner = $null
if (Test-Path -LiteralPath $mermaidPath) {
  $mermaidText = [System.IO.File]::ReadAllText($mermaidPath, [System.Text.Encoding]::UTF8)
  $versionMatch = [regex]::Match($mermaidText, 'name:"mermaid",version:"([^"]+)"')
  if ($versionMatch.Success) {
    $MermaidVersion = $versionMatch.Groups[1].Value
    $MermaidVersionNote = '取自打包文件内的 `name:"mermaid",version:"..."` 字段'
  }

  $bannerMarker = '/*! Bundled license information:'
  $bannerStart = $mermaidText.IndexOf($bannerMarker, [System.StringComparison]::Ordinal)
  if ($bannerStart -ge 0) {
    $bannerEnd = $mermaidText.IndexOf('*/', $bannerStart, [System.StringComparison]::Ordinal)
    if ($bannerEnd -ge 0) {
      $MermaidBanner = Get-NormalizedString -Text $mermaidText.Substring($bannerStart, $bannerEnd - $bannerStart + 2)
      # Keep the upstream text verbatim except for the URL delimiters, so this file has no `<...>`.
      $MermaidBanner = [regex]::Replace($MermaidBanner, '<((?:https?://)[^>\s]+)>', '$1')
    }
  }
}

$QuickJsPackage = $Packages | Where-Object { $_.name -eq 'rquickjs-sys' } | Select-Object -First 1
$QuickJsNote = if ($QuickJsPackage) {
  "经 rquickjs-sys $($QuickJsPackage.version) 捆绑"
} else {
  '经 rquickjs 捆绑，Cargo.lock 未记录具体版本'
}

$QuickJsCopyright = @()
$QuickJsLicenseNote = $null
if ($QuickJsPackage) {
  $quickJsDir = Split-Path -Parent $QuickJsPackage.manifest_path
  $quickJsLicensePath = Join-Path $quickJsDir 'quickjs\LICENSE'
  if (Test-Path -LiteralPath $quickJsLicensePath -PathType Leaf) {
    $quickJsText = Get-NormalizedText -Path $quickJsLicensePath
    $QuickJsCopyright = @($quickJsText -split "`n" | Where-Object { $_ -match '^\s*Copyright' })
    $QuickJsLicenseNote = ('`rquickjs-sys {0}/quickjs/LICENSE`（cargo registry 源目录）' -f $QuickJsPackage.version)
  }
}

$LinkedOmPath = Join-Path $AssetDir 'linkedom.worker.mjs'
$LinkedOmBanner = @()
if (Test-Path -LiteralPath $LinkedOmPath) {
  $linkedOmText = [System.IO.File]::ReadAllText($LinkedOmPath, [System.Text.Encoding]::UTF8)
  $head = $linkedOmText.Substring(0, [Math]::Min(4096, $linkedOmText.Length))
  $LinkedOmBanner = @($head -split "`r?`n" | Where-Object { $_ -match '(?i)copyright|@license|SPDX-License-Identifier' })
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
Add-Line '对于 SPDX OR 表达式，本项目选择白名单内的宽松许可；未选择的许可不构成适用条款。'
Add-Line ''
Add-Line ("- Rust 包（含 workspace 成员）：{0}" -f $Packages.Count)
Add-Line '- 内嵌第三方资产：3（Mermaid、linkedom、QuickJS）'
Add-Line ("- 包根目录找到许可证全文的包：{0}" -f $SourceCounts['root'])
Add-Line ("- 仅在子目录（递归至 3 层）找到全文的包：{0}" -f $SourceCounts['recursive'])
Add-Line ("- 由 manifest `license_file` 指定的全文：{0}" -f $SourceCounts['declared'])
Add-Line ("- 由 workspace 根 LICENSE 覆盖的包：{0}" -f $SourceCounts['workspace'])
Add-Line ("- 无独立许可证文件的包（按 SPDX 标准全文与元数据归属补全）：{0}" -f $NoText.Count)
Add-Line ("- 许可证全文条目（本地文件按内容 SHA-256 去重）：{0}" -f $Sections.Count)
Add-Line ("- 按 SPDX 标准全文补全的许可证组：{0}" -f $GapGroups.Count)
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
Add-Line '以下资产随 `veil.exe` 分发，但不通过 Cargo 依赖引入。版权行一律引自可核验的随包来源；来源中没有版权行时明确写出，不代填版权人。'
Add-Line ''
Add-Line ("### Mermaid {0}" -f $MermaidVersion)
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line '- 来源：`crates/veil-mermaid-offline/assets/mermaid.min.js`'
Add-Line ("- 版本来源：{0}" -f $MermaidVersionNote)
if ($null -ne $MermaidBanner) {
  Add-Line '- 产物内许可声明（原文引自该文件末尾的 `Bundled license information` 注释块；仅将包裹 URL 的尖括号改为圆括号，其余文字未改动）：'
  Add-Line ''
  Add-Line (Get-FencedBlock -Text $MermaidBanner).TrimEnd("`n")
  Add-Line ''
  Add-Line '- Mermaid 自身版权行：上游未在随仓库产物中附带版权行；适用 MIT，版权归属以上游发布为准（https://github.com/mermaid-js/mermaid）。'
} else {
  Add-Line '- 版权行：上游未在随仓库产物中附带版权行；适用 MIT，版权归属以上游发布为准（https://github.com/mermaid-js/mermaid）。'
}
Add-Line ''
Add-Line '### linkedom（bundled）'
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line '- 来源：`crates/veil-mermaid-offline/assets/linkedom.worker.mjs`（构建时由 `build.rs` 转换为经典脚本）'
Add-Line '- 版本：bundled（打包产物未记录上游版本号；以上游项目发布记录为准）'
if ($LinkedOmBanner.Count -gt 0) {
  Add-Line '- 产物内版权行（原文引自上述文件开头的注释）：'
  Add-Line ''
  foreach ($line in $LinkedOmBanner) { Add-Line ('  ' + $line.Trim()) }
  Add-Line ''
  Add-Line '- 上游项目：https://github.com/WebReflection/linkedom'
} else {
  Add-Line '- 版权行：上游未在随仓库产物中附带版权行；适用 MIT，版权归属以上游发布为准（https://github.com/WebReflection/linkedom）。'
}
Add-Line ''
Add-Line '### QuickJS（bundled）'
Add-Line ''
Add-Line '- 许可证：MIT'
Add-Line ('- 来源：' + $QuickJsNote + '；QuickJS 源码经 Rust 依赖 `rquickjs` 编译进 `veil.exe`')
Add-Line '- 版本：bundled（由 rquickjs-sys 随 crate 固定，具体版本见 Cargo.lock）'
if ($QuickJsPackage) {
  $quickJsCrateLicense = if ([string]::IsNullOrWhiteSpace($QuickJsPackage.license)) { '未声明' } else { $QuickJsPackage.license }
  $quickJsCrateRepository = if ([string]::IsNullOrWhiteSpace($QuickJsPackage.repository)) { '未声明' } else { $QuickJsPackage.repository }
  Add-Line '- `rquickjs-sys` crate 元数据：'
  Add-Line ("  - 版本：{0}" -f $QuickJsPackage.version)
  Add-Line ("  - 许可证：{0}" -f $quickJsCrateLicense)
  Add-Line ("  - repository：{0}" -f $quickJsCrateRepository)
  Add-Line ("  - authors：{0}" -f (Format-Authors -Authors $QuickJsPackage.authors))
}
if ($QuickJsCopyright.Count -gt 0) {
  Add-Line ("- 版权行（原文引自 {0}）：" -f $QuickJsLicenseNote)
  Add-Line ''
  foreach ($line in $QuickJsCopyright) { Add-Line ('  ' + $line.Trim()) }
  Add-Line ''
  Add-Line '- 该 crate 以 MIT 分发，捆绑 QuickJS；上述 QuickJS 版权行来自其内置 LICENSE。'
  Add-Line '- 上游项目：https://bellard.org/quickjs/'
} else {
  if ($QuickJsPackage) {
    Add-Line '- 该 crate 以 MIT 分发，捆绑 QuickJS；其内置 LICENSE 未在本地 crate 源目录中找到。'
  }
  Add-Line '- 版权行：上游未在随仓库产物中附带版权行；适用 MIT，版权归属以上游发布为准（https://bellard.org/quickjs/）。'
}
Add-Line ''

Add-Line '## 许可证全文'
Add-Line ''
Add-Line '以下文本来自各包 crate 源目录中的 `LICENSE*`、`LICENCE*`、`COPYING*`、`NOTICE*`、`UNLICENSE*` 文件（优先包根目录，其次最多 3 层子目录，跳过 `target/`）；按内容 SHA-256 去重并按哈希值排序，每份文本前列出适用包与来源相对路径。引用文本中的尖括号定界符统一改为圆括号，括号内文字保持原样。'
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
  Add-Line (Get-FencedBlock -Text $section.Content).TrimEnd("`n")
  Add-Line ''
}

Add-Line '## 无独立许可证文件的包（按 SPDX 标准全文补全）'
Add-Line ''
Add-Line '以下上游包未在 crate 源中附带独立许可证文件；SPDX `OR` 表达式先按宽松许可优先级选定一个适用许可再分组，各包完整表达式见列表，版权归属见包元数据 authors/repository。每个分组附所选许可证的标准全文。authors 为空时记为“未声明”；authors 中的邮箱尖括号和正文中的尖括号定界符在本文件中以圆括号呈现。'
Add-Line ''
if ($GapGroups.Count -eq 0) {
  Add-Line '- 无'
  Add-Line ''
}
foreach ($id in $GapGroups.Keys) {
  Add-Line ("### {0}" -f $id)
  Add-Line ''
  Add-Line ("以下上游包未在 crate 源中附带独立许可证文件；适用 `{0}`（各包完整 SPDX 表达式见列表），版权归属见包元数据 authors/repository。" -f $id)
  Add-Line ''
  foreach ($entry in $GapGroups[$id]) {
    $license = if ([string]::IsNullOrWhiteSpace($entry.License)) { '未声明' } else { $entry.License }
    $repository = if ([string]::IsNullOrWhiteSpace($entry.Repository)) { '未声明' } else { $entry.Repository }
    $authors = Format-Authors -Authors $entry.Authors
    Add-Line ('- `{0}` {1} — SPDX：`{2}` — repository：{3} — authors：{4}' -f $entry.Name, $entry.Version, $license, $repository, $authors)
  }
  Add-Line ''
  if ($StandardLicenseTexts.ContainsKey($id)) {
    Add-Line (Get-FencedBlock -Text $StandardLicenseTexts[$id]).TrimEnd("`n")
  } else {
    Add-Line '本清单未内置该许可证的标准全文；完整条款见 SPDX 许可证列表 https://spdx.org/licenses/ 。'
  }
  Add-Line ''
}
if ($Unspecified.Count -gt 0) {
  Add-Line '### 未声明 SPDX 许可证'
  Add-Line ''
  Add-Line '以下包在 `Cargo.toml` 中未声明许可证表达式，且 crate 源目录未附带许可证文件；本清单不代填许可证与版权人。'
  Add-Line ''
  foreach ($entry in $Unspecified) {
    $repository = if ([string]::IsNullOrWhiteSpace($entry.Repository)) { '未声明' } else { $entry.Repository }
    Add-Line ('- `{0}` {1} — repository：{2} — authors：{3}' -f $entry.Name, $entry.Version, $repository, (Format-Authors -Authors $entry.Authors))
  }
  Add-Line ''
}

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
    Write-Host "OK: THIRD-PARTY-LICENSES.md is up to date ($($Packages.Count) packages, $($NoText.Count) without local full text, $($Sections.Count) deduplicated texts)" -ForegroundColor Green
    exit 0
  }

  $actualText = [System.IO.File]::ReadAllText($OutFile, [System.Text.Encoding]::UTF8)
  $actualLines = $actualText -split "`r?`n"
  $expectedLines = $GeneratedText -split "`n"
  $maxLines = [Math]::Max($actualLines.Count, $expectedLines.Count)
  $diffCount = 0
  $firstDiffLine = -1
  $samples = [System.Collections.Generic.List[string]]::new()
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
$fileSize = (Get-Item -LiteralPath $OutFile).Length
Write-Host "Wrote $OutFile" -ForegroundColor Green
Write-Host ("  packages: {0}; license full text: root={1}, recursive={2}, declared={3}, workspace={4}; without local full text: {5}; deduplicated texts: {6}; spdx groups: {7}; bytes: {8}" -f $Packages.Count, $SourceCounts['root'], $SourceCounts['recursive'], $SourceCounts['declared'], $SourceCounts['workspace'], $NoText.Count, $Sections.Count, $GapGroups.Count, $fileSize)
exit 0
