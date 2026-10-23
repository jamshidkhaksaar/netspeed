//! Per-user Windows startup integration. The registry is the source of truth:
//! loading a missing/corrupt config must never remove a working startup entry.
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
const VALUE_NAME: &str = "NetSpeed";

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn check(rc: WIN32_ERROR) -> io::Result<()> {
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc.0 as i32))
    }
}

fn read_value(subkey: &str) -> io::Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
    let subkey = crate::util::wide(subkey);
    let name = crate::util::wide(VALUE_NAME);
    unsafe {
        let mut raw = HKEY::default();
        let rc = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &mut raw,
        );
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        let key = Key(raw);
        let mut kind = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        let rc = RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut len),
        );
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        if len > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NetSpeed startup entry is unexpectedly large",
            ));
        }
        let mut bytes = vec![0u8; len as usize];
        let rc = RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut len),
        );
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc)?;
        bytes.truncate(len as usize);
        Ok(Some((kind, bytes)))
    }
}

fn decode_string(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16(&wide).ok()
}

fn disabled(bytes: &[u8]) -> bool {
    // Explorer records 2/6 for enabled and 3/7 for disabled startup entries.
    matches!(bytes.first(), Some(3 | 7))
}

/// Read effective Windows state without rewriting it on every app launch.
pub fn is_enabled() -> io::Result<bool> {
    let Some((kind, bytes)) = read_value(RUN_KEY)? else {
        return Ok(false);
    };
    if kind != REG_SZ {
        return Ok(false);
    }
    let Some(command) = decode_string(&bytes) else {
        return Ok(false);
    };
    // Our entry is a quoted, argument-free executable path. A moved/deleted
    // portable exe is not a functioning startup entry.
    let exe = command.trim().trim_matches('"');
    if exe.is_empty() || !Path::new(exe).is_file() {
        return Ok(false);
    }
    Ok(!read_value(APPROVED_KEY)?.is_some_and(|(_, bytes)| disabled(&bytes)))
}

fn command_bytes(exe: &Path) -> Vec<u8> {
    let wide = std::iter::once(b'"' as u16)
        .chain(exe.as_os_str().encode_wide())
        .chain([b'"' as u16, 0]);
    wide.flat_map(u16::to_le_bytes).collect()
}

fn remove_value(subkey: &str) -> io::Result<()> {
    let subkey = crate::util::wide(subkey);
    let name = crate::util::wide(VALUE_NAME);
    unsafe {
        let mut raw = HKEY::default();
        let rc = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut raw,
        );
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(rc)?;
        let key = Key(raw);
        let rc = RegDeleteValueW(key.0, PCWSTR(name.as_ptr()));
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(rc)
    }
}

/// Called only for an explicit user action, never as a side effect of config load.
pub fn set_enabled(enable: bool) -> io::Result<()> {
    if !enable {
        return remove_value(RUN_KEY);
    }
    let exe = std::env::current_exe()?;
    let bytes = command_bytes(&exe);
    let subkey = crate::util::wide(RUN_KEY);
    let name = crate::util::wide(VALUE_NAME);
    unsafe {
        let mut raw = HKEY::default();
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut raw,
            None,
        ))?;
        let key = Key(raw);
        check(RegSetValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            REG_SZ,
            Some(&bytes),
        ))?;
    }
    // An explicit "Start with Windows" action may re-enable this app, but
    // ordinary launches must respect a Task Manager disable. No other app's
    // startup approval entry is changed.
    remove_value(APPROVED_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_spaces_and_preserves_unicode() {
        let exe = Path::new("C:\\Users\\Some User\\速度 📶\\netspeed.exe");
        let bytes = command_bytes(exe);
        assert_eq!(
            decode_string(&bytes).as_deref(),
            Some("\"C:\\Users\\Some User\\速度 📶\\netspeed.exe\"")
        );
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
    }

    #[test]
    fn malformed_registry_strings_are_rejected() {
        assert_eq!(decode_string(&[65]), None);
        assert_eq!(decode_string(&[0, 0xd8]), None);
        assert_eq!(decode_string(&[65, 0, 0, 0, 66, 0]).as_deref(), Some("A"));
    }

    #[test]
    fn task_manager_disabled_states_are_respected() {
        for state in [3, 7] {
            assert!(disabled(&[state, 0, 0, 0]));
        }
        for state in [2, 6] {
            assert!(!disabled(&[state, 0, 0, 0]));
        }
        assert!(!disabled(&[]));
    }
}
