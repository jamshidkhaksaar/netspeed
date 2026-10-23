#requires -Version 5.1
<#
Dependency-free, isolated installer tests. Run:
  pwsh -NoProfile -File .\tests\install.Tests.ps1
  powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\install.Tests.ps1
All files live in a unique tests/.isolated-* subtree. APPDATA/LOCALAPPDATA are
redirected; registry, WScript.Shell, and Start-Process are strict mocks. No app
or installer ever touches the real user profile, registry, shell, or processes.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$global:NetSpeedInstallerTest_InstallerPath = Join-Path (Split-Path $PSScriptRoot -Parent) 'install.ps1'
$global:NetSpeedInstallerTest_TestBase = [System.IO.Path]::GetFullPath($PSScriptRoot)
$global:NetSpeedInstallerTest_TestLeaf = '.isolated-' + [guid]::NewGuid().ToString('N')
$global:NetSpeedInstallerTest_TestRoot = Join-Path $global:NetSpeedInstallerTest_TestBase $global:NetSpeedInstallerTest_TestLeaf
$originalAppData = $env:APPDATA
$originalLocalAppData = $env:LOCALAPPDATA
$global:NetSpeedInstallerTest_CaseNumber = 0
$global:NetSpeedInstallerTest_Passed = 0
$global:NetSpeedInstallerTest_Failures = @()
$global:NetSpeedInstallerTest_RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$global:NetSpeedInstallerTest_ApprovalKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$global:NetSpeedInstallerTest_Utf8 = [System.Text.UTF8Encoding]::new($false)

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}
function Assert-Equal {
    param($Actual, $Expected, [string]$Message)
    if ($Actual -cne $Expected) { throw "$Message`nExpected: $Expected`nActual: $Actual" }
}
function Assert-Throws {
    param([scriptblock]$Body, [string]$Pattern)
    $message = $null
    try { & $Body | Out-Null } catch { $message = $_.Exception.Message }
    Assert-True ($null -ne $message) 'Expected a terminating error.'
    Assert-True ($message -match $Pattern) "Error was not actionable/matching '$Pattern': $message"
    return $message
}
function New-Fixture {
    $global:NetSpeedInstallerTest_CaseNumber++
    $global:NetSpeedInstallerTest_Fixture = Join-Path $global:NetSpeedInstallerTest_TestRoot ('case-' + $global:NetSpeedInstallerTest_CaseNumber)
    $env:LOCALAPPDATA = Join-Path $global:NetSpeedInstallerTest_Fixture 'Local AppData'
    $env:APPDATA = Join-Path $global:NetSpeedInstallerTest_Fixture 'Roaming AppData'
    [void][System.IO.Directory]::CreateDirectory($env:LOCALAPPDATA)
    [void][System.IO.Directory]::CreateDirectory($env:APPDATA)
    $global:NetSpeedInstallerTest_Source = Join-Path $global:NetSpeedInstallerTest_Fixture 'build [release]\source.exe'
    [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_Source))
    [System.IO.File]::WriteAllBytes($global:NetSpeedInstallerTest_Source, [byte[]](77, 90, 1, 2, 3, 4))
    $global:NetSpeedInstallerTest_Destination = Join-Path $env:LOCALAPPDATA 'Programs\NetSpeed\netspeed.exe'
    $global:NetSpeedInstallerTest_ShortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\NetSpeed.lnk'
    $global:NetSpeedInstallerTest_ConfigPath = Join-Path $env:APPDATA 'NetSpeed\config.json'
    $global:NetSpeedInstallerTest_RunExists = $false
    $global:NetSpeedInstallerTest_ApprovalExists = $false
    $global:NetSpeedInstallerTest_ApprovalPresent = $false
    $global:NetSpeedInstallerTest_RegistryWrites = @()
    $global:NetSpeedInstallerTest_SavedShortcuts = [System.Collections.ArrayList]::new()
    $global:NetSpeedInstallerTest_Launches = @()
    $global:NetSpeedInstallerTest_ComCreations = 0
    $global:NetSpeedInstallerTest_CopyAttempts = 0
    $global:NetSpeedInstallerTest_FailCopy = $false
}
function Set-FixtureConfig {
    param([string]$Text)
    [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_ConfigPath))
    [System.IO.File]::WriteAllText($global:NetSpeedInstallerTest_ConfigPath, $Text, $global:NetSpeedInstallerTest_Utf8)
}
function Test-Case {
    param([string]$Name, [scriptblock]$Body)
    try {
        New-Fixture
        & $Body
        $global:NetSpeedInstallerTest_Passed++
        Write-Host "PASS $Name"
    } catch {
        $global:NetSpeedInstallerTest_Failures += "$Name`: $($_.Exception.Message) at $($_.InvocationInfo.ScriptLineNumber)"
        Write-Host "FAIL $Name`: $($_.Exception.Message)"
    }
}

# Strict mocks: never fall through to real registry, COM, or process operations.
function Test-Path {
    [CmdletBinding()]
    param([string]$LiteralPath, [string]$Path, [string]$PathType)
    $target = if ($PSBoundParameters.ContainsKey('LiteralPath')) { $LiteralPath } else { $Path }
    if ($target -like 'HK*:*') {
        if ($target -eq $global:NetSpeedInstallerTest_RunKey) { return $global:NetSpeedInstallerTest_RunExists }
        if ($target -eq $global:NetSpeedInstallerTest_ApprovalKey) { return $global:NetSpeedInstallerTest_ApprovalExists }
        throw "Unexpected registry read: $target"
    }
    Microsoft.PowerShell.Management\Test-Path @PSBoundParameters
}
function New-Item {
    [CmdletBinding()]
    param([string]$Path, [string]$ItemType, [switch]$Force)
    if ($Path -like 'HK*:*') {
        Assert-Equal $Path $global:NetSpeedInstallerTest_RunKey 'Only the HKCU Run key may be created.'
        Assert-True (-not $Force) 'Do not recreate/wipe an existing Run key with -Force.'
        $global:NetSpeedInstallerTest_RunExists = $true
        $global:NetSpeedInstallerTest_RegistryWrites += [pscustomobject]@{ Operation = 'CreateKey'; Path = $Path }
        return
    }
    Microsoft.PowerShell.Management\New-Item @PSBoundParameters
}
function New-ItemProperty {
    [CmdletBinding()]
    param([string]$LiteralPath, [string]$Name, [string]$Value, [string]$PropertyType, [switch]$Force)
    Assert-Equal $LiteralPath $global:NetSpeedInstallerTest_RunKey 'Only HKCU Run may be written.'
    Assert-Equal $Name 'NetSpeed' 'Only the NetSpeed Run value may change.'
    Assert-Equal $PropertyType 'String' 'Run must use REG_SZ.'
    $global:NetSpeedInstallerTest_RegistryWrites += [pscustomobject]@{ Operation = 'SetValue'; Path = $LiteralPath; Name = $Name; Value = $Value }
}
function Get-ItemProperty {
    [CmdletBinding()]
    param([string]$LiteralPath)
    Assert-Equal $LiteralPath $global:NetSpeedInstallerTest_ApprovalKey 'Only app-specific approval key may be inspected.'
    $properties = [pscustomobject]@{ UnrelatedApp = [byte[]](3, 0, 0, 0, 1, 2, 3, 4) }
    if ($global:NetSpeedInstallerTest_ApprovalPresent) {
        Add-Member -InputObject $properties -MemberType NoteProperty -Name NetSpeed -Value ([byte[]](3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))
    }
    return $properties
}
function Remove-ItemProperty {
    [CmdletBinding()]
    param([string]$LiteralPath, [string]$Name)
    Assert-Equal $LiteralPath $global:NetSpeedInstallerTest_ApprovalKey 'Only StartupApproved\\Run may be changed.'
    Assert-Equal $Name 'NetSpeed' 'Never alter unrelated StartupApproved values.'
    $global:NetSpeedInstallerTest_ApprovalPresent = $false
    $global:NetSpeedInstallerTest_RegistryWrites += [pscustomobject]@{ Operation = 'RemoveApproval'; Path = $LiteralPath; Name = $Name }
}
function New-Object {
    [CmdletBinding()]
    param([string]$ComObject)
    Assert-Equal $ComObject 'WScript.Shell' 'Only WScript.Shell is expected.'
    $global:NetSpeedInstallerTest_ComCreations++
    $shell = [pscustomobject]@{
        ExpectedPath = $global:NetSpeedInstallerTest_ShortcutPath
        Records = $global:NetSpeedInstallerTest_SavedShortcuts
        Utf8 = $global:NetSpeedInstallerTest_Utf8
    }
    Add-Member -InputObject $shell -MemberType ScriptMethod -Name CreateShortcut -Value {
        param([string]$Path)
        if ($Path -cne $this.ExpectedPath) { throw "Unexpected shortcut path: $Path" }
        $link = [pscustomobject]@{
            Path = $Path; TargetPath = ''; WorkingDirectory = ''; Arguments = '';
            IconLocation = ''; Description = ''; Records = $this.Records; Utf8 = $this.Utf8
        }
        Add-Member -InputObject $link -MemberType ScriptMethod -Name Save -Value {
            [void]$this.Records.Add([pscustomobject]@{
                Path = $this.Path; TargetPath = $this.TargetPath;
                WorkingDirectory = $this.WorkingDirectory; Arguments = $this.Arguments;
                IconLocation = $this.IconLocation
            })
            [System.IO.File]::WriteAllText($this.Path, ('Mock shortcut: ' + $this.TargetPath), $this.Utf8)
        }
        return $link
    }
    return $shell
}
function Start-Process {
    [CmdletBinding()]
    param([string]$FilePath, [string]$WorkingDirectory)
    Assert-Equal $FilePath $global:NetSpeedInstallerTest_Destination 'Only the stable installed executable may be launched.'
    $global:NetSpeedInstallerTest_Launches += [pscustomobject]@{ FilePath = $FilePath; WorkingDirectory = $WorkingDirectory }
}
function Copy-Item {
    [CmdletBinding()]
    param([string]$LiteralPath, [string]$Destination, [switch]$Force)
    $global:NetSpeedInstallerTest_CopyAttempts++
    if ($global:NetSpeedInstallerTest_FailCopy -and $Destination -eq $global:NetSpeedInstallerTest_Destination) {
        throw 'The process cannot access the file because it is being used by another process.'
    }
    Microsoft.PowerShell.Management\Copy-Item @PSBoundParameters
}

try {
    [void][System.IO.Directory]::CreateDirectory($global:NetSpeedInstallerTest_TestRoot)
    $tokens = $null
    $parseErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($global:NetSpeedInstallerTest_InstallerPath, [ref]$tokens, [ref]$parseErrors)
    Assert-Equal $parseErrors.Count 0 'Installer must pass the PowerShell parser.'
    . $global:NetSpeedInstallerTest_InstallerPath
    Assert-True (-not [System.IO.File]::Exists($global:NetSpeedInstallerTest_Destination)) 'Dot-sourcing must never install.'

    Test-Case 'only root autostart changes; large integers, nested data, strings remain exact' {
        $text = '{"adapter_luid":18446744073709551615,"nested":{"autostart":false},"autostart": false,"custom":"2030-01-02T03:04:05Z","unicode":"\u2603","float":1.2300e+9}'
        Assert-Equal (Get-NetSpeedEnabledConfig -Text $text) ($text.Replace('"autostart": false', '"autostart": true')) 'Config tokens other than autostart must not change.'
        Assert-Equal (Get-NetSpeedEnabledConfig -Text '{"auto\u0073tart":false,"x":1}') '{"auto\u0073tart":true,"x":1}' 'Escaped root keys must not create duplicate fields.'
    }
    Test-Case 'missing autostart is added without replacing existing fields' {
        $text = '{"adapter_name":"Wi-Fi","unknown":{"autostart":false}}'
        $updated = Get-NetSpeedEnabledConfig -Text $text
        $parsed = ConvertFrom-Json -InputObject $updated
        Assert-Equal $parsed.adapter_name 'Wi-Fi' 'Existing adapter name must survive.'
        Assert-True $parsed.autostart 'Root autostart must be enabled.'
        Assert-True (-not $parsed.unknown.autostart) 'Nested autostart must remain false.'
        Assert-True ((ConvertFrom-Json -InputObject (Get-NetSpeedEnabledConfig -Text '{}')).autostart) 'Empty JSON object must work.'
    }
    Test-Case 'fresh script invocation installs stable target and shortcut without launch' {
        $result = & $global:NetSpeedInstallerTest_InstallerPath -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch
        Assert-Equal $result.ExePath $global:NetSpeedInstallerTest_Destination 'Installed path must be stable.'
        Assert-Equal ([Convert]::ToBase64String([System.IO.File]::ReadAllBytes($global:NetSpeedInstallerTest_Destination))) ([Convert]::ToBase64String([System.IO.File]::ReadAllBytes($global:NetSpeedInstallerTest_Source))) 'Executable copy must match.'
        Assert-Equal $global:NetSpeedInstallerTest_SavedShortcuts.Count 1 'Start menu shortcut must be created.'
        $link = $global:NetSpeedInstallerTest_SavedShortcuts[0]
        Assert-Equal $link.TargetPath $global:NetSpeedInstallerTest_Destination 'Shortcut target must be unquoted stable executable path.'
        Assert-Equal $link.WorkingDirectory ([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_Destination)) 'Shortcut working directory must be install directory.'
        Assert-Equal $link.Arguments '' 'Shortcut needs no arguments.'
        Assert-Equal $link.IconLocation ($global:NetSpeedInstallerTest_Destination + ',0') 'Shortcut must use executable icon.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 2 'Missing Run key is created and app value written.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites[1].Value ('"' + $global:NetSpeedInstallerTest_Destination + '"') 'Run path must be properly quoted, including spaces.'
        Assert-Equal ([System.IO.File]::ReadAllBytes($global:NetSpeedInstallerTest_ConfigPath)[0]) 123 'Config must be UTF-8 without BOM.'
        Assert-True ((ConvertFrom-Json -InputObject ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath))).autostart) 'Fresh config must enable autostart.'
        Assert-Equal $global:NetSpeedInstallerTest_Launches.Count 0 '-NoLaunch must never start a process.'
    }
    Test-Case 'default release path is relative to installer, not current directory' {
        $isolatedInstaller = Join-Path $global:NetSpeedInstallerTest_Fixture 'install.ps1'
        Microsoft.PowerShell.Management\Copy-Item -LiteralPath $global:NetSpeedInstallerTest_InstallerPath -Destination $isolatedInstaller
        $releasePath = Join-Path $global:NetSpeedInstallerTest_Fixture 'target\release\netspeed.exe'
        [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($releasePath))
        [System.IO.File]::Copy($global:NetSpeedInstallerTest_Source, $releasePath)
        & $isolatedInstaller -NoLaunch | Out-Null
        Assert-True ([System.IO.File]::Exists($global:NetSpeedInstallerTest_Destination)) 'Default release binary must be installed.'
        Assert-Equal $global:NetSpeedInstallerTest_Launches.Count 0 'Default install supports -NoLaunch.'
    }
    Test-Case 'existing config is preserved and explicit enable clears only NetSpeed approval' {
        $text = '{"autostart":false,"interval_ms":500,"adapter_luid":18446744073709551615,"units":"bits","unknown":{"deep":[1,2,3]}}'
        Set-FixtureConfig $text
        $global:NetSpeedInstallerTest_RunExists = $true
        $global:NetSpeedInstallerTest_ApprovalExists = $true
        $global:NetSpeedInstallerTest_ApprovalPresent = $true
        Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch | Out-Null
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath)) ($text.Replace('"autostart":false', '"autostart":true')) 'Only autostart may change.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 2 'Only app Run write and app approval removal are expected.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites[0].Operation 'SetValue' 'Existing Run key must not be recreated.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites[1].Operation 'RemoveApproval' 'Explicit installation must unblock only NetSpeed.'
        Assert-True (-not $global:NetSpeedInstallerTest_ApprovalPresent) 'NetSpeed approval must be cleared.'
    }
    Test-Case '-NoAutostart leaves even malformed config and existing disabled state untouched' {
        $text = '{"autostart":false, BROKEN'
        Set-FixtureConfig $text
        $global:NetSpeedInstallerTest_RunExists = $true
        $global:NetSpeedInstallerTest_ApprovalExists = $true
        $global:NetSpeedInstallerTest_ApprovalPresent = $true
        $result = Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source -NoAutostart -NoLaunch
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath)) $text 'NoAutostart must not read/rewrite startup config.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 0 'NoAutostart must not alter registry.'
        Assert-True $global:NetSpeedInstallerTest_ApprovalPresent 'Task Manager disable must be preserved without explicit startup enable.'
        Assert-Equal $result.Autostart 'Unchanged' 'Summary must describe preserved startup state.'
    }
    Test-Case 'normal launch is mocked and targets installed binary' {
        Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source | Out-Null
        Assert-Equal $global:NetSpeedInstallerTest_Launches.Count 1 'Default mode requests one launch.'
        Assert-Equal $global:NetSpeedInstallerTest_Launches[0].WorkingDirectory ([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_Destination)) 'Launch working directory must be stable.'
    }
    Test-Case '-WhatIf has no filesystem, registry, shortcut or launch side effects' {
        $text = '{"autostart":false,"show":true}'
        Set-FixtureConfig $text
        & $global:NetSpeedInstallerTest_InstallerPath -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch -WhatIf | Out-Null
        Assert-True (-not [System.IO.File]::Exists($global:NetSpeedInstallerTest_Destination)) 'WhatIf must not copy executable.'
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath)) $text 'WhatIf must preserve config.'
        Assert-Equal $global:NetSpeedInstallerTest_ComCreations 0 'WhatIf must not instantiate shell COM.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 0 'WhatIf must not write registry.'
        Assert-Equal $global:NetSpeedInstallerTest_Launches.Count 0 'WhatIf must not launch.'
    }
    Test-Case 'locked destination fails actionably without changing settings or killing anything' {
        $text = '{"autostart":false,"decimals":2}'
        Set-FixtureConfig $text
        [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_Destination))
        [System.IO.File]::WriteAllText($global:NetSpeedInstallerTest_Destination, 'old executable')
        $global:NetSpeedInstallerTest_FailCopy = $true
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch } 'tray menu.*No processes were stopped' | Out-Null
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_Destination)) 'old executable' 'Locked binary must remain untouched.'
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath)) $text 'Failed copy must preserve config.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 0 'Failed copy must not change startup.'
        Assert-Equal $global:NetSpeedInstallerTest_ComCreations 0 'Failed copy must not touch shell.'
    }
    Test-Case 'invalid config is backed up byte-for-byte and installation aborts' {
        $text = '{"adapter_name":"personal", BROKEN'
        Set-FixtureConfig $text
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch } 'left untouched and backed up.*Fix it or use -NoAutostart' | Out-Null
        Assert-Equal ([System.IO.File]::ReadAllText($global:NetSpeedInstallerTest_ConfigPath)) $text 'Invalid original must never be replaced.'
        $backups = [System.IO.Directory]::GetFiles([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_ConfigPath), 'config.json.invalid-*.bak')
        Assert-Equal $backups.Count 1 'Invalid config needs a distinct backup.'
        Assert-Equal ([System.IO.File]::ReadAllText($backups[0])) $text 'Backup must preserve original bytes/text.'
        Assert-True (-not [System.IO.File]::Exists($global:NetSpeedInstallerTest_Destination)) 'Invalid config must fail before executable installation.'
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 0 'Invalid config must not touch registry.'
    }
    Test-Case '-WhatIf does not create even an invalid-config backup' {
        Set-FixtureConfig '{BROKEN'
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Source -NoLaunch -WhatIf } 'left untouched' | Out-Null
        $backups = [System.IO.Directory]::GetFiles([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_ConfigPath), '*.bak')
        Assert-Equal $backups.Count 0 'WhatIf must not back up/change files.'
    }
    Test-Case 'invalid JSON forms cannot be silently replaced' {
        foreach ($text in @('null', '[]', '{"autostart":1}', '{"autostart":false,"autostart":true}', '{"x":1,}', '{/*comment*/"x":1}')) {
            Assert-Throws { Get-NetSpeedEnabledConfig -Text $text } '.' | Out-Null
        }
    }
    Test-Case 'missing, empty, non-executable and directory sources fail preflight' {
        Assert-Throws { Invoke-NetSpeedInstall -ExePath (Join-Path $global:NetSpeedInstallerTest_Fixture 'missing.exe') -NoLaunch } 'cargo build --release' | Out-Null
        $empty = Join-Path $global:NetSpeedInstallerTest_Fixture 'empty.exe'
        [System.IO.File]::WriteAllBytes($empty, [byte[]]@())
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $empty -NoLaunch } 'nonempty .exe' | Out-Null
        $notExe = Join-Path $global:NetSpeedInstallerTest_Fixture 'readme.txt'
        [System.IO.File]::WriteAllText($notExe, 'not executable')
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $notExe -NoLaunch } 'nonempty .exe' | Out-Null
        Assert-Throws { Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Fixture -NoLaunch } 'nonempty .exe' | Out-Null
        Assert-Equal $global:NetSpeedInstallerTest_RegistryWrites.Count 0 'Preflight failure must not write registry.'
        Assert-Equal $global:NetSpeedInstallerTest_ComCreations 0 'Preflight failure must not touch shell.'
    }
    Test-Case 'using the already-installed executable skips copying' {
        [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($global:NetSpeedInstallerTest_Destination))
        [System.IO.File]::Copy($global:NetSpeedInstallerTest_Source, $global:NetSpeedInstallerTest_Destination)
        $global:NetSpeedInstallerTest_FailCopy = $true
        Invoke-NetSpeedInstall -ExePath $global:NetSpeedInstallerTest_Destination -NoLaunch | Out-Null
        Assert-Equal $global:NetSpeedInstallerTest_CopyAttempts 0 'Same source and destination must not attempt overwrite.'
        Assert-Equal $global:NetSpeedInstallerTest_SavedShortcuts.Count 1 'Same-path repair must still create Start menu link.'
    }
} finally {
    $env:APPDATA = $originalAppData
    $env:LOCALAPPDATA = $originalLocalAppData
    # Verify the resolved absolute deletion target is our exact generated subtree.
    $resolvedRoot = [System.IO.Path]::GetFullPath($global:NetSpeedInstallerTest_TestRoot)
    if ([System.IO.Path]::GetDirectoryName($resolvedRoot) -cne $global:NetSpeedInstallerTest_TestBase -or
        [System.IO.Path]::GetFileName($resolvedRoot) -cne $global:NetSpeedInstallerTest_TestLeaf) {
        throw "Refusing to clean an unexpected path: $resolvedRoot"
    }
    if ([System.IO.Directory]::Exists($resolvedRoot)) {
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}

Write-Host "$global:NetSpeedInstallerTest_Passed passed; $($global:NetSpeedInstallerTest_Failures.Count) failed. Real registry/COM/processes were not used."
if ($global:NetSpeedInstallerTest_Failures.Count -gt 0) { throw ($global:NetSpeedInstallerTest_Failures -join "`n") }
