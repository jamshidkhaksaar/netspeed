use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RC_ENV_VARS: &[&str] = &[
    "NETSPEED_RC",
    "RC",
    "PATH",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "WindowsSdkVerBinPath",
    "WindowsSdkBinPath",
    "WindowsSDK_ExecutablePath_x64",
    "WindowsSDK_ExecutablePath_x86",
    "WindowsSDK_ExecutablePath_arm64",
    "ProgramFiles(x86)",
    "ProgramFiles",
    "ProgramW6432",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/netspeed.ico");
    for name in RC_ENV_VARS {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        panic!(
            "NetSpeed's native Windows resources require an MSVC Windows target (for example x86_64-pc-windows-msvc)"
        );
    }

    let package_version = env::var("CARGO_PKG_VERSION").expect("Cargo package version is missing");
    let version = windows_version(&package_version).unwrap_or_else(|error| panic!("{error}"));
    let version_text = version.map(|part| part.to_string()).join(".");
    let version_numbers = version.map(|part| part.to_string()).join(",");
    let root = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is missing"),
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory is missing"));
    let icon_path = root.join("assets/netspeed.ico");
    if !icon_path.is_file() {
        panic!(
            "NetSpeed icon is missing: {}. Restore the committed assets or run python -B scripts/generate_icon.py with Pillow installed",
            icon_path.display()
        );
    }

    let manifest_path = out_dir.join("netspeed.manifest");
    fs::write(&manifest_path, application_manifest(&version_text))
        .expect("Could not write the NetSpeed compatibility manifest");
    let rc_path = out_dir.join("netspeed.rc");
    let res_path = out_dir.join("netspeed.res");
    let prerelease_flag = if package_version
        .split('+')
        .next()
        .unwrap_or_default()
        .contains('-')
    {
        "0x2L"
    } else {
        "0x0L"
    };
    // Numeric resource IDs/types avoid a dependency on SDK include directories.
    // ICON 1 is both Explorer's application icon and the runtime tray/class icon.
    let resources = format!(
        r#"#pragma code_page(65001)
LANGUAGE 9, 1
1 ICON "{icon}"
1 VERSIONINFO
 FILEVERSION {version_numbers}
 PRODUCTVERSION {version_numbers}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS {prerelease_flag}
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Jamshid Khaksaar\0"
      VALUE "FileDescription", "NetSpeed taskbar network speed widget\0"
      VALUE "FileVersion", "{version_text}\0"
      VALUE "InternalName", "netspeed\0"
      VALUE "LegalCopyright", "Copyright (c) 2026 Jamshid Khaksaar\0"
      VALUE "OriginalFilename", "netspeed.exe\0"
      VALUE "ProductName", "NetSpeed\0"
      VALUE "ProductVersion", "{version_text}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
        icon = rc_escape(&icon_path.to_string_lossy()),
    );
    fs::write(&rc_path, resources).expect("Could not write the NetSpeed resource script");
    let compiler = find_resource_compiler();
    let output = Command::new(&compiler)
        .arg("/nologo")
        .arg("/c65001")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "Could not run Windows SDK resource compiler {}: {error}",
                compiler.display()
            )
        });
    if !output.status.success() {
        panic!(
            "Windows SDK resource compilation failed ({}; {}):\n{}\n{}",
            compiler.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Keep the existing linker-managed PMv2/asInvoker manifest rather than
    // embedding a second RT_MANIFEST resource in the .res file.
    println!("cargo:rustc-link-arg-bins={}", res_path.display());
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
}

fn windows_version(package_version: &str) -> Result<[u16; 4], String> {
    let core = package_version.split(['-', '+']).next().unwrap_or_default();
    let components = core
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>();
    match components {
        Ok(parts) if parts.len() == 3 => Ok([parts[0], parts[1], parts[2], 0]),
        _ => Err(format!(
            "Cargo version '{package_version}' must have three numeric components in the Windows VERSIONINFO range 0..=65535"
        )),
    }
}

fn rc_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn application_manifest(version: &str) -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="NetSpeed" version="__VERSION__" processorArchitecture="*"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>"#.replace("__VERSION__", version)
}

fn executable_path(value: &OsStr) -> Option<PathBuf> {
    let text = value.to_string_lossy();
    let path = PathBuf::from(text.trim().trim_matches('"'));
    if path.is_file() {
        return Some(path);
    }
    if path.components().count() != 1 {
        return None;
    }
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(&path);
        if candidate.is_file() {
            return Some(candidate);
        }
        if candidate.extension().is_none() {
            let candidate = candidate.with_extension("exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn compiler_arches() -> &'static [&'static str] {
    let host = env::var("HOST").unwrap_or_default();
    if host.starts_with("aarch64-") {
        &["arm64", "x64", "x86"]
    } else if host.starts_with("i686-") || host.starts_with("i586-") {
        &["x86"]
    } else {
        &["x64", "x86"]
    }
}

fn compiler_in_bin(bin: &Path) -> Option<PathBuf> {
    for arch in compiler_arches() {
        let candidate = bin.join(arch).join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let candidate = bin.join("rc.exe");
    candidate.is_file().then_some(candidate)
}

fn compiler_in_sdk(root: &Path) -> Option<PathBuf> {
    let bin = root.join("bin");
    if let Ok(version) = env::var("WindowsSDKVersion") {
        if let Some(compiler) = compiler_in_bin(&bin.join(version.trim_end_matches(['\\', '/']))) {
            return Some(compiler);
        }
    }
    let mut versions = fs::read_dir(&bin)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let numbers = entry
                .file_name()
                .to_string_lossy()
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            Some((numbers, entry.path()))
        })
        .collect::<Vec<_>>();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in versions {
        if let Some(compiler) = compiler_in_bin(&path) {
            return Some(compiler);
        }
    }
    compiler_in_bin(&bin)
}

fn find_resource_compiler() -> PathBuf {
    // Explicit overrides are fail-fast: a typo must not silently select a
    // different SDK. Either a full executable path or an executable on PATH works.
    for name in ["NETSPEED_RC", "RC"] {
        if let Some(value) = env::var_os(name).filter(|value| !value.is_empty()) {
            return executable_path(&value).unwrap_or_else(|| panic!("{name} points to an unavailable resource compiler: {}. Set it to the full path to Windows SDK rc.exe", value.to_string_lossy()));
        }
    }
    if let Some(compiler) = executable_path(OsStr::new("rc.exe")) {
        return compiler;
    }
    for name in [
        "WindowsSdkVerBinPath",
        "WindowsSdkBinPath",
        "WindowsSDK_ExecutablePath_x64",
        "WindowsSDK_ExecutablePath_x86",
        "WindowsSDK_ExecutablePath_arm64",
    ] {
        if let Some(value) = env::var_os(name) {
            for bin in env::split_paths(&value) {
                if let Some(compiler) = compiler_in_bin(&bin) {
                    return compiler;
                }
            }
        }
    }
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("WindowsSdkDir") {
        roots.push(PathBuf::from(root));
    }
    for name in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Some(base) = env::var_os(name) {
            for kit in ["10", "8.1"] {
                roots.push(PathBuf::from(&base).join("Windows Kits").join(kit));
            }
        }
    }
    for root in &roots {
        if let Some(compiler) = compiler_in_sdk(root) {
            return compiler;
        }
    }
    let searched = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    panic!(
        "Windows SDK rc.exe was not found on PATH or in SDK roots ({searched}). Install the Windows SDK / Visual Studio C++ build tools, or set NETSPEED_RC to the full path to rc.exe (for example Windows Kits\\10\\bin\\<SDK version>\\x64\\rc.exe)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_versions_map_to_windows_four_part_versions() {
        assert_eq!(windows_version("1.0.1"), Ok([1, 0, 1, 0]));
        assert_eq!(windows_version("2.3.4-beta.1+build.5"), Ok([2, 3, 4, 0]));
        assert_eq!(
            windows_version("65535.65535.65535"),
            Ok([65535, 65535, 65535, 0])
        );
        for invalid in ["", "1.0", "1.0.1.2", "1.0.65536", "a.0.1"] {
            assert!(windows_version(invalid).is_err());
        }
    }

    #[test]
    fn resource_paths_escape_windows_separators_and_quotes() {
        assert_eq!(
            rc_escape(r#"D:\Project Files\netspeed.ico"#),
            r#"D:\\Project Files\\netspeed.ico"#
        );
        assert_eq!(rc_escape("a\"b"), "a\\\"b");
    }

    #[test]
    fn manifest_keeps_invoker_and_per_monitor_v2_compatibility() {
        let manifest = application_manifest("1.0.1.0");
        assert!(manifest.contains("version=\"1.0.1.0\""));
        assert!(manifest.contains("level=\"asInvoker\""));
        assert!(manifest.contains(">PerMonitorV2<"));
        assert!(manifest.contains(">true/pm<"));
        assert!(manifest.contains(">true</longPathAware>"));
        assert!(!manifest.contains("__VERSION__"));
    }
}
