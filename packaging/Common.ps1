# Shared, side-effect-free helpers for packaging and static artifact inspection.
#Requires -Version 7.2
Set-StrictMode -Version Latest

$script:NetSpeedRepositoryRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).ProviderPath
$script:NetSpeedMsiUpgradeCode = '{6EA9AA8B-FA18-5B5F-A16C-43B4F9E1D770}'
$script:NetSpeedBundleUpgradeCode = '{7F7B4573-E88E-5EF3-A69D-4406C9D9125A}'
$script:NetSpeedWixVersion = '4.0.6'

function Assert-NetSpeedVersion {
    param([Parameter(Mandatory)][string]$Version)
    if ($Version -notmatch '^(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]{0,4})$') {
        throw 'Version must be a canonical numeric major.minor.patch, with no suffix or leading zeroes.'
    }
    $parts = $Version.Split('.')
    if ([int]$parts[0] -gt 255 -or [int]$parts[1] -gt 255 -or [int]$parts[2] -gt 65535) {
        throw 'MSI version limits are major <= 255, minor <= 255, patch <= 65535.'
    }
}

function Get-NetSpeedProductCode {
    param([Parameter(Mandatory)][string]$Version, [ValidateSet('x64')][string]$Architecture = 'x64')
    Assert-NetSpeedVersion $Version
    # RFC 4122 UUIDv5: namespace bytes are in network order, not Guid.ToByteArray order.
    $namespaceHex = $script:NetSpeedMsiUpgradeCode.Trim('{}').Replace('-', '')
    $namespaceBytes = [byte[]]::new(16)
    for ($i = 0; $i -lt 16; $i++) {
        $namespaceBytes[$i] = [Convert]::ToByte($namespaceHex.Substring($i * 2, 2), 16)
    }
    $nameBytes = [Text.Encoding]::UTF8.GetBytes("NetSpeed/product/$Architecture/$Version")
    $inputBytes = [byte[]]::new($namespaceBytes.Length + $nameBytes.Length)
    [Array]::Copy($namespaceBytes, 0, $inputBytes, 0, $namespaceBytes.Length)
    [Array]::Copy($nameBytes, 0, $inputBytes, $namespaceBytes.Length, $nameBytes.Length)
    $sha1 = [Security.Cryptography.SHA1]::Create()
    try { $hash = $sha1.ComputeHash($inputBytes) } finally { $sha1.Dispose() }
    $hash[6] = [byte](($hash[6] -band 0x0f) -bor 0x50)
    $hash[8] = [byte](($hash[8] -band 0x3f) -bor 0x80)
    $hex = [Convert]::ToHexString([byte[]]$hash[0..15])
    return ('{' + $hex.Substring(0, 8) + '-' + $hex.Substring(8, 4) + '-' +
        $hex.Substring(12, 4) + '-' + $hex.Substring(16, 4) + '-' + $hex.Substring(20, 12) + '}')
}

function Get-NetSpeedFilePath {
    param([Parameter(Mandatory)][string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or $item.PSProvider.Name -ne 'FileSystem' -or $item.Length -eq 0) {
        throw "Expected a non-empty regular file: $Path"
    }
    return $item.FullName
}

function Get-NetSpeedWritePath {
    param([Parameter(Mandatory)][string]$Path)
    $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
    $resolved = [IO.Path]::GetFullPath($resolved)
    $root = $script:NetSpeedRepositoryRoot.TrimEnd('\', '/')
    if (-not $resolved.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Packaging writes must stay below the repository root, not in a real profile or another checkout: $resolved"
    }
    # Reject junction/symlink parents so a workspace-looking path cannot escape to a profile.
    $ancestor = $resolved
    while ($ancestor -and $ancestor.Length -ge $root.Length) {
        if (Test-Path -LiteralPath $ancestor) {
            $item = Get-Item -LiteralPath $ancestor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Packaging output cannot traverse a reparse point: $ancestor"
            }
        }
        if ($ancestor.Equals($root, [StringComparison]::OrdinalIgnoreCase)) { break }
        $ancestor = Split-Path -Parent $ancestor
    }
    return $resolved
}

function Get-NetSpeedWixPath {
    param([string]$WixPath)
    if (-not $WixPath) { $WixPath = Join-Path $script:NetSpeedRepositoryRoot '.tools\wix\wix.exe' }
    if (Test-Path -LiteralPath $WixPath -PathType Container) { $WixPath = Join-Path $WixPath 'wix.exe' }
    return Get-NetSpeedFilePath $WixPath
}

function Assert-NetSpeedWixVersion {
    param([Parameter(Mandatory)][string]$WixPath)
    $versionOutput = @(& $WixPath --version 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "WiX version query failed with exit code $LASTEXITCODE. $versionOutput" }
    $text = ($versionOutput | ForEach-Object { $_.ToString() }) -join "`n"
    if ($text -notmatch '(?m)^4\.0\.6(?:[.+-][0-9A-Za-z.-]+)?\s*$') {
        throw "Expected pinned WiX $script:NetSpeedWixVersion; received: $text"
    }
}

function Get-NetSpeedWixExtension {
    param([Parameter(Mandatory)][ValidateSet('WixToolset.UI.wixext', 'WixToolset.Bal.wixext')][string]$Name)
    $cache = Join-Path $script:NetSpeedRepositoryRoot ".wix\extensions\$Name\$script:NetSpeedWixVersion"
    if (-not (Test-Path -LiteralPath $cache -PathType Container)) {
        throw "Missing repository-local pinned extension $Name/$script:NetSpeedWixVersion. The build owner must acquire it with wix extension add from the repository root. This script never installs tools."
    }
    $dlls = @(Get-ChildItem -LiteralPath $cache -Recurse -File -Filter "$Name.dll")
    if ($dlls.Count -ne 1) { throw "Expected exactly one $Name.dll in the pinned local cache: $cache" }
    return $dlls[0].FullName
}

function Invoke-NetSpeedWix {
    param([Parameter(Mandatory)][string]$WixPath, [Parameter(Mandatory)][string[]]$Arguments)
    Write-Host ('WiX: ' + ($Arguments -join ' '))
    & $WixPath @Arguments
    if ($LASTEXITCODE -ne 0) { throw "WiX failed with exit code $LASTEXITCODE." }
}

function Get-NetSpeedPeInfo {
    param([Parameter(Mandatory)][string]$Path)
    $Path = Get-NetSpeedFilePath $Path
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw "Not a Windows PE: $Path" }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 64 -or ($peOffset + 24) -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x00004550) {
        throw "Invalid PE header: $Path"
    }
    $machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
    $sectionCount = [BitConverter]::ToUInt16($bytes, $peOffset + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
    $optionalOffset = $peOffset + 24
    if (($optionalOffset + $optionalSize) -gt $bytes.Length -or $optionalSize -lt 96) { throw "Truncated PE optional header: $Path" }
    $magic = [BitConverter]::ToUInt16($bytes, $optionalOffset)
    if ($magic -ne 0x10b -and $magic -ne 0x20b) { throw "Unsupported PE optional header: $Path" }
    $isPlus = $magic -eq 0x20b
    $dataOffset = $optionalOffset + $(if ($isPlus) { 112 } else { 96 })
    $directoryCountOffset = $optionalOffset + $(if ($isPlus) { 108 } else { 92 })
    $directoryCount = [BitConverter]::ToUInt32($bytes, $directoryCountOffset)
    $cliRva = [uint32]0
    $resourceRva = [uint32]0
    if ($directoryCount -gt 14 -and ($dataOffset + 120) -le ($optionalOffset + $optionalSize)) {
        $cliRva = [BitConverter]::ToUInt32($bytes, $dataOffset + 112)
    }
    if ($directoryCount -gt 2 -and ($dataOffset + 24) -le ($optionalOffset + $optionalSize)) {
        $resourceRva = [BitConverter]::ToUInt32($bytes, $dataOffset + 16)
    }
    $sections = @()
    $sectionOffset = $optionalOffset + $optionalSize
    if (($sectionOffset + 40 * $sectionCount) -gt $bytes.Length) { throw "Truncated PE section table: $Path" }
    for ($i = 0; $i -lt $sectionCount; $i++) {
        $at = $sectionOffset + $i * 40
        $sections += [pscustomobject]@{
            VirtualSize = [BitConverter]::ToUInt32($bytes, $at + 8)
            VirtualAddress = [BitConverter]::ToUInt32($bytes, $at + 12)
            RawSize = [BitConverter]::ToUInt32($bytes, $at + 16)
            RawOffset = [BitConverter]::ToUInt32($bytes, $at + 20)
        }
    }
    function Convert-NetSpeedRvaToOffset {
        param([uint32]$Rva)
        foreach ($section in $sections) {
            $size = [Math]::Max([long]$section.VirtualSize, [long]$section.RawSize)
            if ($Rva -ge $section.VirtualAddress -and [long]$Rva -lt ([long]$section.VirtualAddress + $size)) {
                $offset = [long]$section.RawOffset + [long]$Rva - [long]$section.VirtualAddress
                if ($offset -lt 0 -or $offset -ge $bytes.Length) { throw "Invalid PE RVA: $Path" }
                return [int]$offset
            }
        }
        throw "Unmapped PE RVA: $Path"
    }
    $imports = @()
    if ($directoryCount -gt 1 -and ($dataOffset + 16) -le ($optionalOffset + $optionalSize)) {
        $importRva = [BitConverter]::ToUInt32($bytes, $dataOffset + 8)
        if ($importRva -ne 0) {
            $descriptor = Convert-NetSpeedRvaToOffset $importRva
            for ($i = 0; $i -lt 1024; $i++) {
                $at = $descriptor + 20 * $i
                if (($at + 20) -gt $bytes.Length) { throw "Truncated PE import table: $Path" }
                $nameRva = [BitConverter]::ToUInt32($bytes, $at + 12)
                if ($nameRva -eq 0) { break }
                $nameAt = Convert-NetSpeedRvaToOffset $nameRva
                $end = $nameAt
                while ($end -lt $bytes.Length -and $bytes[$end] -ne 0 -and ($end - $nameAt) -lt 4096) { $end++ }
                if ($end -ge $bytes.Length -or $bytes[$end] -ne 0) { throw "Invalid PE import name: $Path" }
                $imports += [Text.Encoding]::ASCII.GetString($bytes, $nameAt, $end - $nameAt)
            }
        }
    }
    $versionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    return [pscustomobject]@{
        Path = $Path
        Machine = ('0x{0:X4}' -f $machine)
        OptionalHeader = ('0x{0:X4}' -f $magic)
        IsNative = $cliRva -eq 0 -and -not ($imports -match '^(mscoree|hostfxr|hostpolicy|coreclr)\.dll$')
        ClrHeaderRva = $cliRva
        HasResources = $resourceRva -ne 0
        Subsystem = [BitConverter]::ToUInt16($bytes, $optionalOffset + 68)
        Imports = $imports
        FileVersion = $versionInfo.FileVersion
        ProductVersion = $versionInfo.ProductVersion
        ProductName = $versionInfo.ProductName
        CompanyName = $versionInfo.CompanyName
    }
}

function Assert-NetSpeedPeVersion {
    param([Parameter(Mandatory)]$PeInfo, [Parameter(Mandatory)][string]$Version)
    $pattern = '^' + [regex]::Escape($Version) + '(?:\.0)?$'
    if ($PeInfo.Machine -ne '0x8664' -or -not $PeInfo.IsNative) { throw "Expected native x64 PE: $($PeInfo.Path)" }
    if ($PeInfo.ProductVersion -notmatch $pattern -or $PeInfo.FileVersion -notmatch $pattern) {
        throw "PE version mismatch for $($PeInfo.Path): product=$($PeInfo.ProductVersion), file=$($PeInfo.FileVersion), expected=$Version"
    }
    if (-not $PeInfo.HasResources) { throw "Expected embedded version/icon resource tree: $($PeInfo.Path)" }
}

function Write-NetSpeedUtf8 {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}
