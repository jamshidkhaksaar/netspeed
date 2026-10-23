# Windows packaging

NetSpeed v1.0.1 ships a portable application, a per-user MSI and a native EXE setup wrapper. The setup wrapper contains the same MSI offered as a separate download.

## Build requirements

- Windows with a Rust MSVC toolchain, Visual Studio C++ Build Tools and Windows SDK resource compiler.
- PowerShell 7.2 or later.
- .NET SDK/runtime 8 for the build tools, with process-local `DOTNET_ROLL_FORWARD=Major` for the pinned WiX 4 tool.
- WiX 4.0.6, `WixToolset.UI.wixext/4.0.6` and `WixToolset.Bal.wixext/4.0.6`.

The installed NetSpeed application and native setup UI do not need .NET. These are unsigned releases; no code-signing certificate is generated or installed.

Run these commands from the repository root in PowerShell 7:

```powershell
$env:DOTNET_ROLL_FORWARD = 'Major'
dotnet tool install wix --version 4.0.6 --tool-path .tools\wix
& .\.tools\wix\wix.exe extension add WixToolset.UI.wixext/4.0.6
& .\.tools\wix\wix.exe extension add WixToolset.Bal.wixext/4.0.6
cargo test --locked
cargo build --release --locked --target-dir target\release-package
$appExe = (Resolve-Path .\target\release-package\release\netspeed.exe).Path
& .\packaging\build-installers.ps1 -Version 1.0.1 -AppExe $appExe
& .\packaging\validate-installers.ps1 -Version 1.0.1 -OutputDir .\dist\v1.0.1
```

[build-installers.ps1](<build-installers.ps1>) accepts `-Version`, an absolute `-AppExe`, and optional `-WixPath` / `-OutputDir`. It consumes a compiled x64 GUI executable with matching version resources. It does not compile Rust, acquire tools, install packages, run NetSpeed, or terminate processes. Output and working directories must stay inside this checkout and may not traverse junctions/symlinks.

The script writes:

- `dist/v1.0.1/netspeed.exe`
- `dist/v1.0.1/NetSpeed-1.0.1-x64.msi`
- `dist/v1.0.1/NetSpeed-1.0.1-Setup.exe`
- `dist/v1.0.1/SHA256SUMS.txt`

The application version comes from [Cargo.toml](<../Cargo.toml>). Keep it aligned with `-Version`. Windows file/product version resources use four parts, such as `1.0.1.0`. MSI product versions use three parts, with major/minor at most 255 and patch at most 65535.

## Installation behavior

Both installers target Windows 11 x64 desktop PCs. They install below `%LOCALAPPDATA%\Programs\NetSpeed` for the current user, add a Start menu shortcut and an Installed apps entry, and preserve configuration and logs in `%APPDATA%\NetSpeed`.

Setup does not launch NetSpeed or enable Start with Windows. Start the installed application from the Start menu and configure startup through its tray menu. When migrating an existing portable startup entry, turn Start with Windows off and back on from the installed instance to update its path.

Exit a running NetSpeed instance before upgrading. The MSI disables Restart Manager application closure and suppresses automatic reboot. Burn controls its own reserved MSI properties; the source does not author `REBOOT` as a chained MSI property.

Turn off Start with Windows before uninstalling if you enabled it. Uninstall removes installer-owned program files, shortcut and HKCU bookkeeping; it keeps user settings and CSV logs. The MSI and bundle have stable upgrade families. Each version/architecture gets a deterministic MSI ProductCode, so local and CI builds agree on product identity.

[NetSpeed.wxs](<NetSpeed.wxs>) defines the MSI. [Bundle.wxs](<Bundle.wxs>) defines the native WiX standard bootstrapper. The root MIT license and [THIRD-PARTY-NOTICES.txt](<THIRD-PARTY-NOTICES.txt>) are included in the payload. The pinned WiX 4.0.6 components use MS-RL, with notices and links to the unmodified source included.

## Validation without installation

[validate-installers.ps1](<validate-installers.ps1>) checks native x64 PE versions, release checksums, MSI identity/scope, Windows 11 conditions, owned HKCU component keys, files/shortcut/icon, embedded cabinet members and sizes. It extracts the setup container as data, checks that its MSI hash matches the standalone MSI, and confirms the native bootstrapper has no CLR host or chained runtime prerequisites.

[ReadOnlyMsi.cs](<ReadOnlyMsi.cs>) opens the database with `MSIDBOPEN_READONLY`. It has no installation-session, configure-product or registry-write APIs. Cabinet inspection uses `FILEOP_SKIP`; failed unsafe-member inspection aborts. Exported streams and a JSON report remain in a unique ignored `packaging/.validation` directory for review. No validation command runs `msiexec /i`, the setup wizard, or the application.

Static validation does not prove a complete interactive install/repair/uninstall cycle. Test those flows in a disposable Windows 11 x64 VM before changing installer behavior for future releases. It also does not sign the files or replace an existing running application.

The [GitHub workflow](<../.github/workflows/windows.yml>) builds and validates Windows artifacts on pushes and pull requests. It uploads CI artifacts but does not overwrite published release downloads. Release publication is a separate maintainer action after verification.
