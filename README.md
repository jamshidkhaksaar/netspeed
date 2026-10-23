<p align="center">
  <img src="assets/netspeed.png" width="88" height="88" alt="NetSpeed icon">
</p>
<h1 align="center">netspeed</h1>

<p align="center">
  Live download and upload speeds on your Windows 11 taskbar. Native Rust with Direct2D and DirectWrite.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows%2011%20x64-0078D4?logo=windows11&logoColor=white&style=flat-square" alt="Windows 11 x64">
  <img src="https://img.shields.io/badge/built%20with-Rust-000000?logo=rust&logoColor=white&style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/version-1.0.1-brightgreen?style=flat-square" alt="v1.0.1">
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT">
</p>

<p align="center">
  <img src="docs/hero.png" width="900" alt="NetSpeed network speed widget beside the Windows taskbar tray">
</p>

## Download and install

Requires Windows 11 x64. Choose a download from [v1.0.1](https://github.com/jamshidkhaksaar/netspeed/releases/tag/v1.0.1):

| Download | Purpose |
|---|---|
| [NetSpeed-1.0.1-Setup.exe](https://github.com/jamshidkhaksaar/netspeed/releases/download/v1.0.1/NetSpeed-1.0.1-Setup.exe) | Native setup wizard with the MSI included. |
| [NetSpeed-1.0.1-x64.msi](https://github.com/jamshidkhaksaar/netspeed/releases/download/v1.0.1/NetSpeed-1.0.1-x64.msi) | Windows Installer package for direct installation or deployment. |
| [netspeed.exe](https://github.com/jamshidkhaksaar/netspeed/releases/download/v1.0.1/netspeed.exe) | Portable application. Put it in a folder of your choice and run it. |
| [SHA256SUMS.txt](https://github.com/jamshidkhaksaar/netspeed/releases/download/v1.0.1/SHA256SUMS.txt) | Checksums for the release downloads. |

Exit any running NetSpeed instance from its tray menu before upgrading or switching between portable and installed copies.

The installers install for the current user in `%LOCALAPPDATA%\Programs\NetSpeed` and create a Start menu shortcut. They do not enable autostart by default. Start NetSpeed from the Start menu, then use **Start with Windows** in its tray menu if you want it at logon. If a startup entry already points to a portable copy, turn that option off and back on from the installed instance to update the path.

Windows may put the tray icon in the overflow menu. Open the tray overflow and drag the NetSpeed icon onto the taskbar.

The downloads are unsigned. Windows may display a SmartScreen or unknown-publisher warning. Use the release files from this repository and verify their checksums:

```powershell
Get-FileHash .\NetSpeed-1.0.1-Setup.exe -Algorithm SHA256
```

To uninstall an installed copy, use Windows **Installed apps**. Turn off **Start with Windows** in NetSpeed first if you enabled it. Uninstallation keeps your configuration and CSV logs.

## Features

- Download and upload rates, with update intervals from 250 ms to 5 seconds in the tray menu.
- Automatic selection of an active best-route adapter, including VPN adapters, or manual adapter selection.
- Bits/s, SI bytes/s and binary bytes/s, with precision limited to 0, 1 or 2 decimal places.
- Session traffic totals in the tray tooltip.
- Optional daily CSV files with configurable retention.
- Per-Monitor V2 DPI awareness and Windows system light/dark theme colors.
- A click-through taskbar overlay that does not take keyboard focus.
- An original multi-resolution icon for the application, tray and installed shortcuts.
- Per-user MSI and EXE installers, or a portable executable without an application runtime dependency.

Automatic best-route discovery currently uses IPv4. For an IPv6-only connection or a different IPv6 VPN route, select the required adapter manually. On systems with many virtual/filter interfaces, adapters beyond the tray menu's first 20 entries can be pinned through `adapter_luid` in the configuration.

## Screenshot

<p align="center">
  <img src="docs/widget-zoom.png" width="480" alt="Close-up of the taskbar download and upload rates">
</p>

## Settings

Right-click the tray icon to change settings:

| Menu item | Options |
|---|---|
| Update interval | 250 ms / 500 ms / 1 s / 2 s / 5 s |
| Units | bits/s / bytes/s SI / bytes/s binary |
| Decimals | 0 / 1 / 2 |
| Adapter | Automatic or a listed adapter |
| Save samples to CSV | Daily files with retention |
| Start with Windows | Enable or disable the current executable's startup entry |
| Other commands | Pause, show widget, open logs folder, exit |

Configuration lives in `%APPDATA%\NetSpeed\config.json`; CSV logs live in `%APPDATA%\NetSpeed\logs`. For a custom update interval from 250 ms to 60 seconds, exit the app, edit the configuration, and restart it:

```json
{
  "interval_ms": 500,
  "units": "bytes_bin",
  "decimals": 1,
  "adapter_luid": null,
  "logging": false,
  "log_retention_days": 30,
  "autostart": false,
  "show": true
}
```

Windows startup registration is the source of truth for autostart. Editing the `autostart` field alone does not create or remove a startup entry.

## Build from source

Use a current Rust MSVC toolchain, Visual Studio Build Tools and the Windows SDK. The resource compiler embeds the application icon and version information. [build.rs](<build.rs>) discovers `rc.exe` from the SDK, `PATH`, or an explicit `NETSPEED_RC`/`RC` environment variable.

```powershell
git clone https://github.com/jamshidkhaksaar/netspeed.git
cd netspeed
cargo test --locked
cargo build --release --locked
# Application: target\release\netspeed.exe
```

The icon source is [assets/netspeed.svg](<assets/netspeed.svg>). The committed PNG and ICO assets can be regenerated with [scripts/generate_icon.py](<scripts/generate_icon.py>) and Python with Pillow.

For MSI and EXE packaging, use PowerShell 7.2 or later and the pinned WiX toolchain described in the [packaging guide](<packaging/README.md>). Packaging needs .NET 8 on the build machine; the installed app and native setup wizard do not require .NET.

```powershell
$env:DOTNET_ROLL_FORWARD = 'Major'
dotnet tool install wix --version 4.0.6 --tool-path .tools\wix
& .\.tools\wix\wix.exe extension add WixToolset.UI.wixext/4.0.6
& .\.tools\wix\wix.exe extension add WixToolset.Bal.wixext/4.0.6
cargo build --release --locked --target-dir target\release-package
& .\packaging\build-installers.ps1 -Version 1.0.1 -AppExe (Resolve-Path .\target\release-package\release\netspeed.exe).Path
& .\packaging\validate-installers.ps1 -Version 1.0.1 -OutputDir .\dist\v1.0.1
```

The [isolated installer-script tests](<tests/install.Tests.ps1>) use temporary files and mocked registry, shell and launch operations:

```powershell
pwsh -NoProfile -File .\tests\install.Tests.ps1
```

The optional [PowerShell installer](<install.ps1>) is also available for a locally built executable. Its default behavior explicitly enables per-user autostart; use `-NoAutostart` to preserve startup settings and `-NoLaunch` to avoid launching the app.

```powershell
.\install.ps1 -NoAutostart -NoLaunch
```

Development diagnostics:

```powershell
netspeed.exe --debug-anchor
netspeed.exe --debug-net
netspeed.exe --snap out.bmp
```

## How it works

NetSpeed creates a layered child window of `Shell_TrayWnd`. Direct2D renders into a premultiplied-alpha bitmap, DirectWrite draws the text, and `UpdateLayeredWindow` updates the overlay. The widget follows compatible taskbar geometry, DPI and theme changes.

`GetIfTable2` provides octet counters. The sampler calculates rates from actual elapsed time and tracks the adapter LUID, avoiding invalid deltas after route changes or counter resets. Interval-only changes preserve the sampling baseline. Real suspend/resume events clear it; ordinary battery and A/C notifications do not.

See [CHANGELOG.md](<CHANGELOG.md>) for release changes.

## License

[MIT](<LICENSE>), copyright 2026 Jamshid Khaksaar.

Inspired by [TrafficMonitor](https://github.com/zhongyang219/TrafficMonitor) and [NetSpeedTray](https://github.com/erez-c137/NetSpeedTray). NetSpeed is an original implementation, not a fork of either project.
