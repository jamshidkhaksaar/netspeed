use crate::config::{Config, Units};
use crate::net::AdapterRow;
use windows::core::{BOOL, Free, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, SetBitmapBits};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4, NOTIFY_ICON_DATA_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, GetCursorPos, HICON, HMENU, ICONINFO,
    LoadIconW, SetForegroundWindow, TrackPopupMenu, IDI_APPLICATION, MENU_ITEM_FLAGS,
    MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN,
    TPM_RIGHTBUTTON,
};

pub const CMD_PAUSE: u32 = 1;
pub const CMD_SHOW: u32 = 2;
pub const CMD_EXIT: u32 = 3;
pub const CMD_OPEN_LOGS: u32 = 4;
pub const CMD_LOGGING: u32 = 5;
pub const CMD_AUTOSTART: u32 = 6;
pub const CMD_INTERVAL_250: u32 = 10;
pub const CMD_INTERVAL_500: u32 = 11;
pub const CMD_INTERVAL_1000: u32 = 12;
pub const CMD_INTERVAL_2000: u32 = 13;
pub const CMD_INTERVAL_5000: u32 = 14;
pub const CMD_UNITS_BITS: u32 = 20;
pub const CMD_UNITS_SI: u32 = 21;
pub const CMD_UNITS_BIN: u32 = 22;
pub const CMD_DEC_0: u32 = 30;
pub const CMD_DEC_1: u32 = 31;
pub const CMD_DEC_2: u32 = 32;
pub const CMD_ADAPTER_AUTO: u32 = 40;
pub const CMD_ADAPTER_BASE: u32 = 100;

pub struct Tray {
    pub hwnd: HWND,
    icon: Option<HICON>,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let icon = make_icon().or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION).ok() });
        Tray {
            hwnd,
            icon,
            added: false,
        }
    }

    pub fn add_icon(&mut self) {
        let mut d = nid(self.hwnd, NIF_MESSAGE | NIF_ICON | NIF_TIP);
        d.uCallbackMessage = crate::TRAY_CALLBACK_MSG;
        if let Some(i) = self.icon {
            d.hIcon = i;
        }
        set_tip(&mut d, "NetSpeed");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &d);
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        }
        self.added = true;
    }

    pub fn re_add(&mut self) {
        self.added = false;
        self.add_icon();
    }

    pub fn update_tip(&self, tip: &str) {
        if !self.added {
            return;
        }
        let mut d = nid(self.hwnd, NIF_TIP);
        set_tip(&mut d, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = nid(self.hwnd, NIF_MESSAGE);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }

    pub fn shutdown(&mut self) {
        self.remove();
        if let Some(mut i) = self.icon.take() {
            unsafe {
                i.free();
            }
        }
    }
}

fn nid(hwnd: HWND, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
    let mut d = NOTIFYICONDATAW::default();
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d.uFlags = flags;
    d
}

fn set_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    let w = crate::util::utf16(tip);
    let n = w.len().min(d.szTip.len() - 1);
    d.szTip[..n].copy_from_slice(&w[..n]);
    d.szTip[n] = 0;
}

fn make_icon() -> Option<HICON> {
    unsafe {
        let sz: i32 = 16;
        let mut pixels = vec![0u32; (sz * sz) as usize];
        for y in 2..14i32 {
            let t = 13 - y;
            let hw = (t + 1) / 2;
            for x in (8 - hw).max(0)..(8 + hw).min(sz) {
                pixels[(y * sz + x) as usize] = 0xFF_60CD_FF;
            }
        }
        let color = CreateBitmap(sz, sz, 1, 32, Some(pixels.as_ptr() as *const core::ffi::c_void));
        if color.0.is_null() {
            return None;
        }
        let mask = CreateBitmap(sz, sz, 1, 1, None);
        if mask.0.is_null() {
            let mut c = color;
            c.free();
            return None;
        }
        let zeros = [0u8; 32];
        let _ = SetBitmapBits(mask, 32, zeros.as_ptr() as *const core::ffi::c_void);
        let info = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        CreateIconIndirect(&info).ok()
    }
}

fn ck(checked: bool) -> MENU_ITEM_FLAGS {
    if checked {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    }
}

fn append(m: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str) {
    let w = crate::util::wide(text);
    unsafe {
        let _ = AppendMenuW(m, flags, id, PCWSTR(w.as_ptr()));
    }
}

fn append_sub(m: HMENU, sub: HMENU, text: &str) {
    let w = crate::util::wide(text);
    unsafe {
        let _ = AppendMenuW(m, MF_POPUP, sub.0 as usize, PCWSTR(w.as_ptr()));
    }
}

pub fn show_menu(hwnd: HWND, cfg: &Config, adapters: &[AdapterRow], paused: bool) -> u32 {
    unsafe {
        let mut menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return 0,
        };
        append(
            menu,
            MF_STRING,
            CMD_PAUSE as usize,
            if paused { "Resume" } else { "Pause" },
        );
        append(menu, ck(cfg.show), CMD_SHOW as usize, "Show widget");
        append(menu, MF_SEPARATOR, 0, "");

        let sub_i = CreatePopupMenu().unwrap_or_default();
        for (id, label, ms) in [
            (CMD_INTERVAL_250, "250 ms", 250u32),
            (CMD_INTERVAL_500, "500 ms", 500),
            (CMD_INTERVAL_1000, "1 s", 1000),
            (CMD_INTERVAL_2000, "2 s", 2000),
            (CMD_INTERVAL_5000, "5 s", 5000),
        ] {
            append(sub_i, ck(cfg.interval_ms == ms), id as usize, label);
        }
        append_sub(menu, sub_i, "Update interval");

        let sub_u = CreatePopupMenu().unwrap_or_default();
        append(
            sub_u,
            ck(cfg.units == Units::Bits),
            CMD_UNITS_BITS as usize,
            "bits/s (Kb/s)",
        );
        append(
            sub_u,
            ck(cfg.units == Units::BytesSi),
            CMD_UNITS_SI as usize,
            "bytes/s, SI (KB/s)",
        );
        append(
            sub_u,
            ck(cfg.units == Units::BytesBin),
            CMD_UNITS_BIN as usize,
            "bytes/s, binary (KiB/s)",
        );
        append_sub(menu, sub_u, "Units");

        let sub_d = CreatePopupMenu().unwrap_or_default();
        for (id, label, d) in [
            (CMD_DEC_0, "0", 0u32),
            (CMD_DEC_1, "1", 1),
            (CMD_DEC_2, "2", 2),
        ] {
            append(sub_d, ck(cfg.decimals == d), id as usize, label);
        }
        append_sub(menu, sub_d, "Decimals");

        let sub_a = CreatePopupMenu().unwrap_or_default();
        append(
            sub_a,
            ck(cfg.adapter_luid.is_none()),
            CMD_ADAPTER_AUTO as usize,
            "Automatic",
        );
        for (i, r) in adapters.iter().take(20).enumerate() {
            let name = if !r.alias.is_empty() {
                r.alias.clone()
            } else {
                r.description.clone()
            };
            let name = if name.chars().count() > 48 {
                let mut s: String = name.chars().take(45).collect();
                s.push_str("...");
                s
            } else {
                name
            };
            append(
                sub_a,
                ck(cfg.adapter_luid == Some(r.luid)),
                CMD_ADAPTER_BASE as usize + i,
                &name,
            );
        }
        append_sub(menu, sub_a, "Adapter");

        append(menu, MF_SEPARATOR, 0, "");
        append(
            menu,
            ck(cfg.logging),
            CMD_LOGGING as usize,
            "Save samples to CSV",
        );
        append(
            menu,
            ck(cfg.autostart),
            CMD_AUTOSTART as usize,
            "Start with Windows",
        );
        append(menu, MF_STRING, CMD_OPEN_LOGS as usize, "Open logs folder");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_EXIT as usize, "Exit");

        let mut pt = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        menu.free();
        cmd.0 as u32
    }
}
