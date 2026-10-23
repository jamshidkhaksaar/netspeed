#Requires -Version 7.2
<#
.SYNOPSIS
Build the portable EXE, a per-user MSI, and a native Burn setup EXE.
.DESCRIPTION
Consumes an already compiled native x64 application. Does not compile Rust,
acquire tools, install either package, run the application, or validate via an
installer session. Only WiX 4.0.6 CLI build operations run. All write paths are
inside this checkout; temporary working files stay under packaging/.build.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$AppExe,
    [string]$WixPath,
    [string]$OutputDir
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows is required to build these Windows installers.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
Assert-NetSpeedVersion $Version
if (-not [IO.Path]::IsPathFullyQualified($AppExe)) { throw 'AppExe must be an absolute path to the compiled application EXE.' }
$app = Get-NetSpeedFilePath $AppExe
$appInfo = Get-NetSpeedPeInfo $app
Assert-NetSpeedPeVersion -PeInfo $appInfo -Version $Version
if ($appInfo.Subsystem -ne 2) { throw 'The release application must use the Windows GUI subsystem.' }

$wix = Get-NetSpeedWixPath $WixPath
$uiExtension = Get-NetSpeedWixExtension 'WixToolset.UI.wixext'
$balExtension = Get-NetSpeedWixExtension 'WixToolset.Bal.wixext'
$icon = Get-NetSpeedFilePath (Join-Path $script:NetSpeedRepositoryRoot 'assets\netspeed.ico')
$logo = Get-NetSpeedFilePath (Join-Path $script:NetSpeedRepositoryRoot 'assets\netspeed.png')
$license = Get-NetSpeedFilePath (Join-Path $script:NetSpeedRepositoryRoot 'LICENSE')
$licenseText = [IO.File]::ReadAllText($license)
if ($licenseText -notmatch '(?m)^MIT License\s*$' -or $licenseText -notmatch '(?m)^Copyright \(c\) 2026 Jamshid Khaksaar\s*$') {
    throw 'Expected the root MIT LICENSE with copyright 2026 Jamshid Khaksaar.'
}
if (-not $OutputDir) { $OutputDir = Join-Path $script:NetSpeedRepositoryRoot "dist\v$Version" }
$output = Get-NetSpeedWritePath $OutputDir
$work = Get-NetSpeedWritePath (Join-Path $PSScriptRoot ('.build\v' + $Version + '-' + [guid]::NewGuid().ToString('N')))
$msiWork = Join-Path $work 'msi'
$bundleWork = Join-Path $work 'bundle'
$licenseRtf = Join-Path $work 'License.rtf'
$portable = Join-Path $output 'netspeed.exe'
$msi = Join-Path $output "NetSpeed-$Version-x64.msi"
$bundle = Join-Path $output "NetSpeed-$Version-Setup.exe"
$productCode = Get-NetSpeedProductCode $Version

# These are process-only variables, restored in finally. Never set user/machine
# environment variables or write to the registry/profile to configure .NET.
$savedRollForward = $env:DOTNET_ROLL_FORWARD
$savedTemp = $env:TEMP
$savedTmp = $env:TMP
Push-Location $script:NetSpeedRepositoryRoot
try {
    $env:DOTNET_ROLL_FORWARD = 'Major'
    New-Item -ItemType Directory -Path $output, $work, $msiWork, $bundleWork -Force | Out-Null
    $env:TEMP = $work
    $env:TMP = $work
    Assert-NetSpeedWixVersion $wix

    # Minimal RTF generated from the actual root LICENSE, not a second copy.
    $escaped = [Text.StringBuilder]::new()
    foreach ($character in $licenseText.ToCharArray()) {
        switch ([int]$character) {
            13 { }
            10 { [void]$escaped.Append("\par`n") }
            92 { [void]$escaped.Append('\\') }
            123 { [void]$escaped.Append('\{') }
            125 { [void]$escaped.Append('\}') }
            default {
                $code = [int]$character
                if ($code -lt 128) { [void]$escaped.Append($character) }
                else {
                    if ($code -gt 32767) { $code -= 65536 }
                    [void]$escaped.Append(('\u{0}?' -f $code))
                }
            }
        }
    }
    $rtf = '{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Segoe UI;}}\viewkind4\uc1\pard\f0\fs18 ' + $escaped.ToString() + '}'
    Write-NetSpeedUtf8 -Path $licenseRtf -Text $rtf

    $msiArguments = @(
        'build', (Join-Path $PSScriptRoot 'NetSpeed.wxs'),
        '-arch', 'x64', '-culture', 'en-US', '-ext', $uiExtension,
        '-d', "Version=$Version", '-d', "ProductCode=$productCode",
        '-d', "AppExe=$app", '-d', "IconFile=$icon",
        '-d', "LicenseFile=$license", '-d', "LicenseRtf=$licenseRtf",
        '-d', "NoticesFile=$(Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt')",
        '-intermediateFolder', $msiWork,
        '-pdb', (Join-Path $msiWork 'NetSpeed.wixpdb'), '-o', $msi
    )
    # WiX 4 CLI build does not run ICE/installer-engine validation. Do not add
    # the obsolete WiX 3 -sval switch or call wix msi validate in this script.
    Invoke-NetSpeedWix -WixPath $wix -Arguments $msiArguments
    [void](Get-NetSpeedFilePath $msi)

    $bundleArguments = @(
        'build', (Join-Path $PSScriptRoot 'Bundle.wxs'),
        '-arch', 'x64', '-ext', $balExtension,
        '-d', "Version=$Version", '-d', "MsiFile=$msi",
        '-d', "IconFile=$icon", '-d', "LogoFile=$logo", '-d', "LicenseRtf=$licenseRtf",
        '-d', "LicenseFile=$license", '-d', "NoticesFile=$(Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt')",
        '-intermediateFolder', $bundleWork,
        '-pdb', (Join-Path $bundleWork 'NetSpeed.wixpdb'), '-o', $bundle
    )
    Invoke-NetSpeedWix -WixPath $wix -Arguments $bundleArguments
    [void](Get-NetSpeedFilePath $bundle)

    if (-not $app.Equals($portable, [StringComparison]::OrdinalIgnoreCase)) {
        Copy-Item -LiteralPath $app -Destination $portable -Force
    }
    $sourceHash = (Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash
    if ((Get-FileHash -LiteralPath $portable -Algorithm SHA256).Hash -ne $sourceHash) { throw 'Portable copy does not match the compiled application.' }
    $files = @($portable, $msi, $bundle) | Sort-Object { [IO.Path]::GetFileName($_) }
    $sumLines = foreach ($file in $files) {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant(), [IO.Path]::GetFileName($file)
    }
    Write-NetSpeedUtf8 -Path (Join-Path $output 'SHA256SUMS.txt') -Text (($sumLines -join "`n") + "`n")
    Write-Host "Built native Windows installers. ProductCode: $productCode"
    Write-Host "Artifacts: $output"
    Write-Host "Intermediates: $work"
    Write-Host 'Not installed, not launched, and not yet validated. Run packaging/validate-installers.ps1 separately.'
}
finally {
    $env:DOTNET_ROLL_FORWARD = $savedRollForward
    $env:TEMP = $savedTemp
    $env:TMP = $savedTmp
    Pop-Location
}
