#requires -Version 5.1
<#
.SYNOPSIS
Installs NetSpeed for the current Windows user; no administrator rights required.
.DESCRIPTION
Copies the release executable to %LOCALAPPDATA%\Programs\NetSpeed, creates a
per-user Start menu shortcut, and explicitly enables Start with Windows.
Existing config values are preserved; only the top-level autostart flag changes.
Explicit enable removes only NetSpeed's StartupApproved\Run value, if present,
so a previous Task Manager disable does not block the requested installation.
No processes are ever stopped. Exit NetSpeed from its tray menu before upgrading
if Windows reports that the installed executable is in use.
.PARAMETER ExePath
Existing built executable. Defaults to target\release\netspeed.exe beside this script.
.PARAMETER NoAutostart
Do not change config, the Run value, or StartupApproved. This does not disable an
existing startup entry; it leaves the user's current startup settings untouched.
.PARAMETER NoLaunch
Install without starting the executable (recommended for build/review automation).
.EXAMPLE
.\install.ps1 -NoLaunch
.EXAMPLE
.\install.ps1 -ExePath 'C:\Downloads\netspeed.exe' -NoLaunch
.EXAMPLE
.\install.ps1 -NoLaunch -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [ValidateNotNullOrEmpty()]
    [string]$ExePath = (Join-Path $PSScriptRoot 'target\release\netspeed.exe'),
    [switch]$NoAutostart,
    [switch]$NoLaunch
)

function Get-NetSpeedEnabledConfig {
    [CmdletBinding()]
    param([AllowEmptyString()][string]$Text)

    # Validate, but never serialize the parsed object: doing so can round u64
    # adapter IDs, coerce date-like strings, or lose unknown/deep config fields.
    $parsed = ConvertFrom-Json -InputObject $Text -ErrorAction Stop
    if ($null -eq $parsed -or $parsed -isnot [pscustomobject] -or
        -not $Text.TrimStart().StartsWith('{')) {
        throw 'Config must be a JSON object.'
    }

    $tokens = [regex]::Matches($Text,
        '"(?:\\.|[^"\\])*"|[{}\[\]:,]|true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?')
    $depth = 0
    $previousEnd = 0
    $memberCount = 0
    $autostartToken = $null
    $closingIndex = -1
    for ($i = 0; $i -lt $tokens.Count; $i++) {
        $token = $tokens[$i]
        # PowerShell 7 accepts comments/trailing commas, but the Rust app does not.
        if ($Text.Substring($previousEnd, $token.Index - $previousEnd) -notmatch '^[ \t\r\n]*$') {
            throw 'Config contains non-JSON text (comments are not supported).'
        }
        $previousEnd = $token.Index + $token.Length
        if ($token.Value -eq '{' -or $token.Value -eq '[') { $depth++; continue }
        if ($token.Value -eq '}' -or $token.Value -eq ']') {
            if ($i -gt 0 -and $tokens[$i - 1].Value -eq ',') {
                throw 'Config contains a trailing comma.'
            }
            $depth--
            if ($depth -eq 0) { $closingIndex = $token.Index }
            continue
        }
        if ($depth -eq 1 -and $token.Value.StartsWith('"') -and
            $i + 1 -lt $tokens.Count -and $tokens[$i + 1].Value -eq ':') {
            $memberCount++
            $key = ConvertFrom-Json -InputObject $token.Value -ErrorAction Stop
            if ($key -ceq 'autostart') {
                if ($null -ne $autostartToken) { throw 'Config has duplicate autostart fields.' }
                $autostartToken = $tokens[$i + 2]
                if ($autostartToken.Value -cne 'true' -and $autostartToken.Value -cne 'false') {
                    throw 'Config autostart must be a JSON boolean.'
                }
            }
        }
    }
    if ($closingIndex -lt 0 -or $depth -ne 0 -or
        $Text.Substring($previousEnd) -notmatch '^[ \t\r\n]*$') {
        throw 'Config is not a complete JSON object.'
    }
    if ($null -ne $autostartToken) {
        return $Text.Remove($autostartToken.Index, $autostartToken.Length).Insert($autostartToken.Index, 'true')
    }
    $separator = if ($memberCount -gt 0) { ',' } else { '' }
    return $Text.Insert($closingIndex, "$separator`n  `"autostart`": true`n")
}

function Write-NetSpeedConfig {
    [CmdletBinding()]
    param([string]$Path, [string]$Text)

    # Same-directory atomic replacement; UTF-8 without BOM is required by serde_json.
    $tempPath = Join-Path ([System.IO.Path]::GetDirectoryName($Path)) (
        '.config-install-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $encoding = [System.Text.UTF8Encoding]::new($false)
        [System.IO.File]::WriteAllText($tempPath, $Text, $encoding)
        if ([System.IO.File]::Exists($Path)) {
            [System.IO.File]::Replace($tempPath, $Path, [NullString]::Value)
        } else {
            [System.IO.File]::Move($tempPath, $Path)
        }
    } finally {
        # Only the exact temporary file generated above is eligible for cleanup.
        if ([System.IO.File]::Exists($tempPath)) { [System.IO.File]::Delete($tempPath) }
    }
}

function Invoke-NetSpeedInstall {
    [CmdletBinding(SupportsShouldProcess = $true)]
    param(
        [ValidateNotNullOrEmpty()]
        [string]$ExePath = (Join-Path $PSScriptRoot 'target\release\netspeed.exe'),
        [switch]$NoAutostart,
        [switch]$NoLaunch
    )

    if ($env:OS -ne 'Windows_NT') { throw 'NetSpeed installation requires Windows.' }
    foreach ($variable in @('LOCALAPPDATA', 'APPDATA')) {
        $value = [Environment]::GetEnvironmentVariable($variable)
        if ([string]::IsNullOrWhiteSpace($value) -or -not [System.IO.Path]::IsPathRooted($value)) {
            throw "The $variable environment variable must point to an absolute per-user directory."
        }
    }
    try { $source = Get-Item -LiteralPath $ExePath -ErrorAction Stop } catch {
        throw "Built executable not found at '$ExePath'. Run cargo build --release first, or pass -ExePath to netspeed.exe. $($_.Exception.Message)"
    }
    if ($source -isnot [System.IO.FileInfo] -or $source.Extension -ine '.exe' -or $source.Length -eq 0) {
        throw "ExePath must be an existing, nonempty .exe file: '$ExePath'."
    }

    $installDir = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\NetSpeed'))
    $destination = Join-Path $installDir 'netspeed.exe'
    $menuDir = [System.IO.Path]::GetFullPath((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'))
    $shortcutPath = Join-Path $menuDir 'NetSpeed.lnk'
    $configDir = [System.IO.Path]::GetFullPath((Join-Path $env:APPDATA 'NetSpeed'))
    $configPath = Join-Path $configDir 'config.json'
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $approvalKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    if (Test-Path -LiteralPath $destination -PathType Container) {
        throw "A directory exists where the installed executable should be: '$destination'. Rename it and retry."
    }

    $configText = $null
    if (-not $NoAutostart) {
        if (Test-Path -LiteralPath $configPath -PathType Container) {
            throw "A directory exists where the config file should be: '$configPath'. Rename it and retry."
        }
        if ([System.IO.File]::Exists($configPath)) {
            try {
                $configText = Get-NetSpeedEnabledConfig -Text ([System.IO.File]::ReadAllText(
                    $configPath, [System.Text.UTF8Encoding]::new($false, $true)))
            } catch {
                $reason = $_.Exception.Message
                $backupPath = $configPath + '.invalid-' + [guid]::NewGuid().ToString('N') + '.bak'
                if ($PSCmdlet.ShouldProcess($backupPath, 'Back up invalid config without changing the original')) {
                    try { [System.IO.File]::Copy($configPath, $backupPath) } catch {
                        throw "Invalid config '$configPath' was left untouched; backup failed: $($_.Exception.Message). Fix the config or use -NoAutostart. Original error: $reason"
                    }
                    throw "Invalid config '$configPath' was left untouched and backed up to '$backupPath'. Fix it or use -NoAutostart, then retry. $reason"
                }
                throw "Invalid config '$configPath' was left untouched. Fix it or use -NoAutostart, then retry. $reason"
            }
        } else {
            $configText = "{`n  `"autostart`": true`n}`n"
        }
    }
    if (-not $PSCmdlet.ShouldProcess($destination, 'Install NetSpeed, create Start menu shortcut, and apply requested startup settings')) {
        return
    }

    $shell = $null
    $shortcut = $null
    try {
        New-Item -ItemType Directory -Path $installDir -Force -ErrorAction Stop | Out-Null
        if (-not [string]::Equals($source.FullName, $destination, [StringComparison]::OrdinalIgnoreCase)) {
            try { Copy-Item -LiteralPath $source.FullName -Destination $destination -Force -ErrorAction Stop } catch {
                throw "Cannot copy NetSpeed to '$destination'. If it is running, choose Exit in its tray menu, then retry with -NoLaunch. No processes were stopped. Check folder permissions/free disk space. $($_.Exception.Message)"
            }
        }
        New-Item -ItemType Directory -Path $menuDir -Force -ErrorAction Stop | Out-Null
        try {
            $shell = New-Object -ComObject WScript.Shell -ErrorAction Stop
            $shortcut = $shell.CreateShortcut($shortcutPath)
            $shortcut.TargetPath = $destination
            $shortcut.WorkingDirectory = $installDir
            $shortcut.Arguments = ''
            $shortcut.IconLocation = "$destination,0"
            $shortcut.Description = 'NetSpeed - live network speed on the taskbar'
            $shortcut.Save()
        } catch {
            throw "Cannot create the per-user Start menu shortcut '$shortcutPath'. Check permissions and WScript.Shell availability, then retry. $($_.Exception.Message)"
        }
        if (-not $NoAutostart) {
            # Write config first, so a registry failure cannot leave a startup
            # entry paired with a deliberately false config value.
            New-Item -ItemType Directory -Path $configDir -Force -ErrorAction Stop | Out-Null
            Write-NetSpeedConfig -Path $configPath -Text $configText
            try {
                if (-not (Test-Path -LiteralPath $runKey)) {
                    New-Item -Path $runKey -ErrorAction Stop | Out-Null
                }
                New-ItemProperty -LiteralPath $runKey -Name NetSpeed -Value ('"' + $destination + '"') -PropertyType String -Force -ErrorAction Stop | Out-Null
                if (Test-Path -LiteralPath $approvalKey) {
                    $approval = Get-ItemProperty -LiteralPath $approvalKey -ErrorAction Stop
                    if ($null -ne $approval.PSObject.Properties['NetSpeed']) {
                        Remove-ItemProperty -LiteralPath $approvalKey -Name NetSpeed -ErrorAction Stop
                        Write-Verbose 'Explicit startup enable cleared only NetSpeed StartupApproved\Run; unrelated entries were preserved.'
                    }
                }
            } catch {
                throw "Installed NetSpeed, but could not enable per-user Windows startup. Retry this installer with -NoLaunch; no administrator rights should be needed. $($_.Exception.Message)"
            }
        }
        if (-not $NoLaunch) {
            try { Start-Process -FilePath $destination -WorkingDirectory $installDir -ErrorAction Stop | Out-Null } catch {
                throw "NetSpeed is installed, but could not be started. Open '$shortcutPath' manually. $($_.Exception.Message)"
            }
        }
        [pscustomobject]@{
            ExePath = $destination
            ShortcutPath = $shortcutPath
            ConfigPath = $configPath
            Autostart = if ($NoAutostart) { 'Unchanged' } else { 'Enabled' }
            Launched = -not $NoLaunch
        }
    } finally {
        if ($null -ne $shortcut -and [System.Runtime.InteropServices.Marshal]::IsComObject($shortcut)) {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
        if ($null -ne $shell -and [System.Runtime.InteropServices.Marshal]::IsComObject($shell)) {
            [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

# Dot-sourcing exposes functions for isolated tests; it never installs or launches.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-NetSpeedInstall @PSBoundParameters
}
