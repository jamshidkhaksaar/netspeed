#Requires -Version 7.2
<# Static inspection only: never install, launch the application, change registry
   settings, or close processes. All exports stay in a unique workspace folder. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$OutputDir,
    [string]$WixPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Static Windows Installer inspection requires Windows.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
Assert-NetSpeedVersion $Version
function Assert-Artifact { param([bool]$Condition, [string]$Message); if (-not $Condition) { throw $Message } }
$output = Get-NetSpeedWritePath $OutputDir
$portable = Get-NetSpeedFilePath (Join-Path $output 'netspeed.exe')
$msi = Get-NetSpeedFilePath (Join-Path $output "NetSpeed-$Version-x64.msi")
$bundle = Get-NetSpeedFilePath (Join-Path $output "NetSpeed-$Version-Setup.exe")
$sumFile = Get-NetSpeedFilePath (Join-Path $output 'SHA256SUMS.txt')
$expectedNames = @('netspeed.exe', "NetSpeed-$Version-x64.msi", "NetSpeed-$Version-Setup.exe")
$hashes = @{}
foreach ($line in [IO.File]::ReadAllLines($sumFile)) {
    Assert-Artifact ($line -match '^([0-9a-fA-F]{64})  ([^\\/:]+)$') "Invalid checksum line: $line"
    $name = $Matches[2]
    Assert-Artifact ($name -in $expectedNames -and -not $hashes.ContainsKey($name)) "Unexpected or duplicate checksum: $name"
    $hashes[$name] = $Matches[1]
    $actual = (Get-FileHash -LiteralPath (Join-Path $output $name) -Algorithm SHA256).Hash
    Assert-Artifact ($actual -eq $Matches[1]) "Checksum mismatch: $name"
}
Assert-Artifact ($hashes.Count -eq 3) 'All three release files must have checksums.'
$appInfo = Get-NetSpeedPeInfo $portable
$bundleInfo = Get-NetSpeedPeInfo $bundle
Assert-NetSpeedPeVersion $appInfo $Version
Assert-NetSpeedPeVersion $bundleInfo $Version
Assert-Artifact ($appInfo.Subsystem -eq 2 -and $bundleInfo.Subsystem -eq 2) 'Expected native Windows GUI executables.'
$work = Get-NetSpeedWritePath (Join-Path $PSScriptRoot ('.validation\v' + $Version + '-' + [guid]::NewGuid().ToString('N')))
New-Item -ItemType Directory -Path $work | Out-Null
if (-not ('NetSpeedPackaging.ReadOnlyMsi' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'ReadOnlyMsi.cs') }
$reader = [NetSpeedPackaging.ReadOnlyMsi]::new($msi)
try {
    $properties = @{}
    foreach ($row in $reader.ReadTable('Property', [string[]]@('Property','Value')).Rows) { $properties[$row['Property']] = $row['Value'] }
    Assert-Artifact ($properties['ProductName'] -eq 'NetSpeed') 'MSI product name is incorrect.'
    Assert-Artifact ($properties['ProductVersion'] -eq $Version) 'MSI version is incorrect.'
    Assert-Artifact ($properties['ProductCode'] -eq (Get-NetSpeedProductCode $Version)) 'MSI ProductCode is not the stable version/architecture identity.'
    Assert-Artifact ($properties['UpgradeCode'] -eq $script:NetSpeedMsiUpgradeCode) 'MSI upgrade family changed.'
    Assert-Artifact (-not $properties.ContainsKey('ALLUSERS') -or [string]::IsNullOrEmpty($properties['ALLUSERS'])) 'MSI must remain per-user.'
    Assert-Artifact ($properties['MSIRESTARTMANAGERCONTROL'] -eq 'Disable') 'MSI must not close running applications automatically.'
    Assert-Artifact ($properties['REBOOT'] -eq 'ReallySuppress') 'MSI must suppress automatic reboot.'
    $summary = $reader.ReadSummary()
    Assert-Artifact ($summary['7'] -eq 'x64;1033') 'Expected x64 English MSI summary.'
    Assert-Artifact (([int]$summary['15'] -band 8) -ne 0) 'MSI summary must declare limited privileges.'
    $fileRows = $reader.ReadTable('File', [string[]]@('File','FileName','FileSize','Version','Component_')).Rows
    $expectedFiles = @{
        NetSpeedExe = 'netspeed.exe'; NetSpeedLicenseFile = 'LICENSE.txt';
        NetSpeedIconFile = 'netspeed.ico'; NetSpeedNoticesFile = 'THIRD-PARTY-NOTICES.txt'
    }
    Assert-Artifact ($fileRows.Count -eq 4) 'MSI must contain the four expected application/license/icon/notice files.'
    $sizes = @{}
    foreach ($row in $fileRows) {
        $id = $row['File']; $name = $row['FileName'].Split('|')[-1]
        Assert-Artifact ($expectedFiles.ContainsKey($id) -and $expectedFiles[$id] -eq $name) "Unexpected MSI file: $id / $name"
        $sizes[$id] = [long]$row['FileSize']
        if ($id -eq 'NetSpeedExe') { Assert-Artifact ($row['Version'] -eq "$Version.0") 'Installed EXE version is incorrect.' }
    }
    $registry = @{}
    foreach ($row in $reader.ReadTable('Registry', [string[]]@('Registry','Root','Key','Name','Component_')).Rows) {
        Assert-Artifact ($row['Root'] -eq '1' -and $row['Key'] -eq 'Software\NetSpeed\Installer') 'Installer must write only its own HKCU bookkeeping keys.'
        $registry[$row['Registry']] = $row
    }
    foreach ($row in $reader.ReadTable('Component', [string[]]@('Component','Attributes','KeyPath')).Rows) {
        Assert-Artifact (([int]$row['Attributes'] -band 256) -ne 0 -and ([int]$row['Attributes'] -band 4) -ne 0) 'Per-user components need x64 HKCU registry key paths.'
        Assert-Artifact ($registry.ContainsKey($row['KeyPath'])) 'Component key path is not owned HKCU bookkeeping.'
    }
    $directories = @{}
    foreach ($row in $reader.ReadTable('Directory', [string[]]@('Directory','Directory_Parent','DefaultDir')).Rows) { $directories[$row['Directory']] = $row }
    Assert-Artifact ($directories['INSTALLFOLDER']['Directory_Parent'] -eq 'NetSpeedProgramsFolder') 'Unexpected install directory.'
    Assert-Artifact ($directories['NetSpeedProgramsFolder']['Directory_Parent'] -eq 'LocalAppDataFolder') 'Application must install below LocalAppData.'
    $shortcuts = $reader.ReadTable('Shortcut', [string[]]@('Name','Target','Icon_','WkDir','Directory_')).Rows
    Assert-Artifact ($shortcuts.Count -eq 1) 'Expected one Start menu shortcut.'
    Assert-Artifact ($shortcuts[0]['Target'] -eq '[#NetSpeedExe]' -and $shortcuts[0]['WkDir'] -eq 'INSTALLFOLDER' -and $shortcuts[0]['Icon_'] -eq 'NetSpeedIcon.ico') 'Start menu target/icon is incorrect.'
    $conditions = $reader.ReadTable('LaunchCondition', [string[]]@('Condition','Description')).Rows
    $osConditions = @($conditions | Where-Object { $_['Condition'] -match 'NETSPEED_OS_BUILD\s*>=\s*22000' -and $_['Condition'] -match 'Msix64' -and $_['Condition'] -match 'MsiNTProductType\s*=\s*1' })
    Assert-Artifact ($osConditions.Count -eq 1) 'Expected Windows 11 x64 desktop launch condition.'
    $locators = $reader.ReadTable('RegLocator', [string[]]@('Root','Key','Name','Type')).Rows
    Assert-Artifact (@($locators | Where-Object { $_['Name'] -eq 'CurrentBuildNumber' -and $_['Root'] -eq '2' -and ([int]$_['Type'] -band 16) -ne 0 }).Count -eq 1) 'OS detection must read the 64-bit Windows build value.'
    foreach ($row in $reader.ReadTable('Upgrade', [string[]]@('UpgradeCode')).Rows) { Assert-Artifact ($row['UpgradeCode'] -eq $script:NetSpeedMsiUpgradeCode) 'Unexpected upgrade relation.' }
    foreach ($row in $reader.ReadTable('RemoveFile', [string[]]@('FileName','InstallMode')).Rows) {
        Assert-Artifact ([string]::IsNullOrEmpty($row['FileName']) -and $row['InstallMode'] -eq '2') 'Uninstall cleanup must remove only empty owned folders.'
    }
    $iconExport = Join-Path $work 'installed-icon.ico'
    $reader.ExportBinary('Icon','Name','NetSpeedIcon.ico','Data',$iconExport)
    Assert-Artifact ((Get-FileHash $iconExport -Algorithm SHA256).Hash -eq (Get-FileHash (Join-Path $script:NetSpeedRepositoryRoot 'assets\netspeed.ico') -Algorithm SHA256).Hash) 'MSI icon differs from the original artwork.'
    $media = $reader.ReadTable('Media', [string[]]@('Cabinet')).Rows
    Assert-Artifact ($media.Count -eq 1 -and $media[0]['Cabinet'].StartsWith('#')) 'Expected one embedded MSI cabinet.'
    $cabinet = Join-Path $work 'application.cab'
    $reader.ExportStream($media[0]['Cabinet'].Substring(1), $cabinet)
    $members = [NetSpeedPackaging.ReadOnlyMsi]::InspectCabinet($cabinet)
    Assert-Artifact ($members.Count -eq 4) 'Embedded cabinet member count is incorrect.'
    foreach ($member in $members) { Assert-Artifact ($sizes.ContainsKey($member.Name) -and $sizes[$member.Name] -eq [long]$member.Size) "Unexpected cabinet member or size: $($member.Name)" }
}
finally { $reader.Dispose() }
$wix = Get-NetSpeedWixPath $WixPath
$savedRollForward = $env:DOTNET_ROLL_FORWARD
$savedTemp = $env:TEMP; $savedTmp = $env:TMP
$content = Join-Path $work 'bundle-content'; $ba = Join-Path $work 'bundle-ba'
Push-Location $script:NetSpeedRepositoryRoot
try {
    $env:DOTNET_ROLL_FORWARD = 'Major'; $env:TEMP = $work; $env:TMP = $work
    Assert-NetSpeedWixVersion $wix
    $wixWork = Join-Path $work 'wix'
    New-Item -ItemType Directory -Path $content, $ba, $wixWork -Force | Out-Null
    Invoke-NetSpeedWix -WixPath $wix -Arguments @('burn','extract',$bundle,'-o',$content,'-oba',$ba,'-intermediateFolder',$wixWork)
    $embeddedMsi = @(Get-ChildItem -LiteralPath $content -Recurse -File -Filter '*.msi')
    Assert-Artifact ($embeddedMsi.Count -eq 1) 'Setup must embed exactly one MSI.'
    Assert-Artifact ((Get-FileHash $embeddedMsi[0].FullName -Algorithm SHA256).Hash -eq (Get-FileHash $msi -Algorithm SHA256).Hash) 'Embedded MSI differs from the standalone download.'
    $nativeBa = @(Get-ChildItem -LiteralPath $ba -Recurse -File | Where-Object { $_.Name -match '(wixstdba|WixStandardBootstrapperApplication).*\.(dll|exe)$' })
    Assert-Artifact ($nativeBa.Count -eq 1) 'Expected one native WiX standard bootstrapper application.'
    $baInfo = Get-NetSpeedPeInfo $nativeBa[0].FullName
    Assert-Artifact ($baInfo.IsNative -and $baInfo.Machine -eq '0x8664') 'Setup UI must be native x64 without a CLR host.'
    $xmlFiles = @(Get-ChildItem -LiteralPath $ba -Recurse -File -Filter '*.xml')
    $packageEntries = @()
    foreach ($file in $xmlFiles) {
        try {
            [xml]$document = [IO.File]::ReadAllText($file.FullName)
            $packageEntries += @($document.SelectNodes("//*[local-name()='WixPackageProperties']"))
        } catch [System.Xml.XmlException] { }
    }
    Assert-Artifact ($packageEntries.Count -eq 1 -and $packageEntries[0].Package -eq 'NetSpeedMsi') 'Setup must chain only the NetSpeed MSI, without runtime prerequisites.'
    $report = [ordered]@{ Version=$Version; ProductCode=$properties['ProductCode']; PerUser=$true; Files=$expectedNames; Hashes=$hashes; App=$appInfo; Setup=$bundleInfo; BootstrapperApplication=$baInfo; InstallationExecuted=$false }
    Write-NetSpeedUtf8 (Join-Path $work 'validation.json') ($report | ConvertTo-Json -Depth 8)
    Write-Host "PASS: portable EXE, per-user MSI, native setup, payload identity and checksums validated for $Version."
    Write-Host "Static report and exports: $work"
    Write-Host 'No installation, application launch, registry change, or process termination was performed.'
}
finally {
    $env:DOTNET_ROLL_FORWARD=$savedRollForward; $env:TEMP=$savedTemp; $env:TMP=$savedTmp
    Pop-Location
}
