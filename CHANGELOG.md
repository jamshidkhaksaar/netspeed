# Changelog

## 1.0.1

### Added

- Original download/upload icon with multiple Windows icon sizes, embedded in the application, tray and installer shortcuts.
- Application file-version and product-version resources derived from the Cargo package version.
- Per-user Windows 11 x64 MSI and native EXE installers, with a Start menu shortcut and uninstall/repair support.
- Packaging scripts, static installer validation and downloadable SHA-256 checksums.

### Fixed

- Restored the missing automatic/manual adapter picker and failed tray-registration retry helper.
- Enabled standard hover tooltips when using notification-icon version 4.
- Preserved session traffic across update-interval changes, including transitions from long custom intervals.
- Stopped ordinary battery and A/C power-status notifications from clearing the sampling baseline.
- Pruned expired CSV files during daily rotation, including after a long logging gap.
- Limited decimal precision when reading configuration and formatting rates to prevent excessive allocations.

### Startup and installation

- Installers preserve startup preferences by default and keep user configuration/logs during uninstall.
- The existing PowerShell installer and Windows startup integration are included in source. Startup state follows the Windows registry instead of a configuration flag alone.

## 1.0.0

Initial native Windows 11 taskbar network-speed widget, with Direct2D/DirectWrite rendering, adapter selection, configurable units and intervals, CSV logging and a tray menu.
