# NetSpeed v1.0.1

This release adds an original Windows application/tray icon and per-user Windows 11 x64 installers, together with the network-sampling fixes.

## Downloads

- `NetSpeed-1.0.1-Setup.exe`: native setup wizard with the MSI embedded.
- `NetSpeed-1.0.1-x64.msi`: standalone per-user Windows Installer package.
- `netspeed.exe`: portable native application.
- `SHA256SUMS.txt`: SHA-256 checksums for all three downloads.

The app installs below `%LOCALAPPDATA%\Programs\NetSpeed` with a Start menu shortcut. Setup does not launch NetSpeed or enable autostart. Use the tray menu to enable Start with Windows after installation. Exit an existing instance before upgrading. Configuration and logs are retained during uninstall; disable Start with Windows first if you enabled it.

## Changes

- New vector download/upload icon with nine Windows icon sizes and DPI-aware tray replacement.
- Embedded file/product version `1.0.1.0` and the existing Per-Monitor V2/asInvoker manifest.
- Restored adapter picking and failed tray-registration retry.
- Restored standard tray hover tooltips.
- Preserved session traffic when changing update intervals, including long custom intervals.
- Ignored routine battery/A/C notifications when deciding whether to reset sampling.
- Enforced CSV retention during daily rotation.
- Clamped configured and formatted decimal precision to 0–2.
- Added reproducible installer build scripts and Windows CI artifact validation.

## Verification and limits

55 application regression tests, 3 resource-build tests and 14 isolated PowerShell installer tests passed. Static package checks verified versions, per-user scope, payload files, shortcut/icon, embedded cabinet contents, native bootstrapper and equality between the embedded and standalone MSI. Executable resource inspection verified all nine icon sizes and the versioned DPI/privilege manifest.

The release is unsigned. Windows may show SmartScreen or unknown-publisher warnings. No installation or repair/uninstall cycle was run on the maintainer's live machine; interactive installer testing should use a disposable Windows 11 x64 VM. The build and validation commands did not stop applications or change startup settings.
