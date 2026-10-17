use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

/// True when Windows is in dark app/system mode (taskbar dark).
pub fn is_dark() -> bool {
    unsafe {
        let mut val: u32 = 0;
        let mut cb: u32 = std::mem::size_of::<u32>() as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut _),
            Some(&mut cb),
        );
        if rc == ERROR_SUCCESS { val == 0 } else { true }
    }
}
