use std::time::{Duration, Instant};

use crate::config::{Config, Units};
use crate::net::AdapterRow;
use windows::Win32::Foundation::{HINSTANCE, HWND, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CopyIcon, CreatePopupMenu, DestroyIcon, FindWindowW, GetCursorPos, HICON, HMENU,
    IDI_APPLICATION, IMAGE_FLAGS, IMAGE_ICON, LoadIconW, LoadImageW, MENU_ITEM_FLAGS, MF_CHECKED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
};
use windows::core::{Free, PCWSTR, w};

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

const ADD_RETRY_INTERVAL: Duration = Duration::from_secs(1);

// Unlike LoadIconW's shared class handle, a tray LoadImageW handle is owned.
// RAII also releases it if startup exits before the normal shutdown path.
struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

pub struct Tray {
    pub hwnd: HWND,
    icon: Option<OwnedIcon>,
    icon_size: (i32, i32),
    added: bool,
    last_add_attempt: Option<Instant>,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let icon_size = tray_icon_size(hwnd);
        let icon = load_owned_icon(icon_size);
        Tray {
            hwnd,
            icon,
            icon_size,
            added: false,
            last_add_attempt: None,
        }
    }

    pub fn add_icon(&mut self) {
        self.ensure_added();
    }

    pub fn ensure_added(&mut self) {
        self.refresh_icon();
        self.ensure_added_with(Instant::now(), notify_icon);
    }

    fn refresh_icon(&mut self) {
        self.refresh_icon_with(tray_icon_size(self.hwnd), load_owned_icon, notify_icon);
    }

    // Keep the previous icon alive until Explorer accepts a DPI replacement.
    // Inject the loader and notifier so tests never create icons or call Shell.
    fn refresh_icon_with(
        &mut self,
        size: (i32, i32),
        load: impl FnOnce((i32, i32)) -> Option<OwnedIcon>,
        mut notify: impl FnMut(NOTIFY_ICON_MESSAGE, &NOTIFYICONDATAW) -> bool,
    ) {
        if size == self.icon_size {
            return;
        }
        let Some(icon) = load(size) else {
            return;
        };
        if self.added {
            let mut data = nid(self.hwnd, NIF_ICON);
            data.hIcon = icon.0;
            if !notify(NIM_MODIFY, &data) {
                return;
            }
        }
        self.icon = Some(icon);
        self.icon_size = size;
    }

    // Inject time and notifications so registration tests never call the Shell.
    fn ensure_added_with(
        &mut self,
        now: Instant,
        mut notify: impl FnMut(NOTIFY_ICON_MESSAGE, &NOTIFYICONDATAW) -> bool,
    ) {
        if self.added
            || self
                .last_add_attempt
                .is_some_and(|last| now.saturating_duration_since(last) < ADD_RETRY_INTERVAL)
        {
            return;
        }

        let mut d = nid(self.hwnd, NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        d.uCallbackMessage = crate::TRAY_CALLBACK_MSG;
        if let Some(icon) = &self.icon {
            d.hIcon = icon.0;
        }
        set_tip(&mut d, "NetSpeed");
        self.last_add_attempt = Some(now);
        if !notify(NIM_ADD, &d) {
            return;
        }

        // A version negotiation failure does not undo a successful registration.
        self.added = true;
        self.last_add_attempt = None;
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = notify(NIM_SETVERSION, &d);
    }

    pub fn re_add(&mut self) {
        self.added = false;
        self.refresh_icon();
        self.re_add_with(Instant::now(), notify_icon);
    }

    fn re_add_with(
        &mut self,
        now: Instant,
        notify: impl FnMut(NOTIFY_ICON_MESSAGE, &NOTIFYICONDATAW) -> bool,
    ) {
        // Explorer lost the previous registration; bypass any failed-add cooldown.
        self.added = false;
        self.last_add_attempt = None;
        self.ensure_added_with(now, notify);
    }

    pub fn update_tip(&self, tip: &str) {
        self.update_tip_with(tip, notify_icon);
    }

    fn update_tip_with(
        &self,
        tip: &str,
        mut notify: impl FnMut(NOTIFY_ICON_MESSAGE, &NOTIFYICONDATAW) -> bool,
    ) {
        if !self.added {
            return;
        }
        let mut d = nid(self.hwnd, NIF_TIP | NIF_SHOWTIP);
        set_tip(&mut d, tip);
        let _ = notify(NIM_MODIFY, &d);
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = nid(self.hwnd, NIF_MESSAGE);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
        self.last_add_attempt = None;
    }

    pub fn shutdown(&mut self) {
        self.remove();
        self.icon = None;
    }
}

fn notify_icon(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
    unsafe { Shell_NotifyIconW(message, data).as_bool() }
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

fn icon_dimensions(dpi: u32, width: i32, height: i32) -> (i32, i32) {
    let fallback = ((16 * u64::from(dpi.max(96)) + 48) / 96).clamp(16, 256) as i32;
    let dimension = |metric: i32| {
        if metric > 0 {
            metric.min(256)
        } else {
            fallback
        }
    };
    (dimension(width), dimension(height))
}

fn tray_icon_size(hwnd: HWND) -> (i32, i32) {
    unsafe {
        // The hidden owner window may be on a different monitor. Explorer's
        // notification area follows the primary taskbar, so use its DPI first.
        let dpi = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())
            .ok()
            .map(|taskbar| GetDpiForWindow(taskbar))
            .filter(|&dpi| dpi != 0)
            .unwrap_or_else(|| GetDpiForWindow(hwnd));
        let dpi = if dpi == 0 {
            GetDpiForSystem().max(96)
        } else {
            dpi
        };
        icon_dimensions(
            dpi,
            GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            GetSystemMetricsForDpi(SM_CYSMICON, dpi),
        )
    }
}

fn load_owned_icon(size: (i32, i32)) -> Option<OwnedIcon> {
    unsafe {
        let resource = GetModuleHandleW(PCWSTR::null()).and_then(|module| {
            // No LR_SHARED: this handle belongs only to Tray and is destroyed
            // after replacement/shutdown. Windows selects the nearest ICO frame.
            LoadImageW(
                Some(HINSTANCE(module.0)),
                crate::app_icon_resource_name(),
                IMAGE_ICON,
                size.0,
                size.1,
                IMAGE_FLAGS(0),
            )
        });
        match resource {
            Ok(handle) => Some(OwnedIcon(HICON(handle.0))),
            Err(error) => {
                crate::util::log_error(&format!("cannot load NetSpeed tray icon: {error}"));
                // Predefined system icons are shared. Copy before taking
                // ownership, never destroy the shared LoadIconW result.
                LoadIconW(None, IDI_APPLICATION)
                    .and_then(|shared| CopyIcon(shared))
                    .ok()
                    .map(OwnedIcon)
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_tray() -> Tray {
        // Do not use Tray::new: constructing a test tray must not create an OS icon.
        Tray {
            hwnd: HWND::default(),
            icon: None,
            icon_size: (16, 16),
            added: false,
            last_add_attempt: None,
        }
    }

    #[test]
    fn icon_size_fallback_matches_native_windows_scaling() {
        for (dpi, size) in [
            (0, 16),
            (96, 16),
            (120, 20),
            (144, 24),
            (192, 32),
            (240, 40),
            (288, 48),
            (384, 64),
            (768, 128),
            (1536, 256),
        ] {
            assert_eq!(icon_dimensions(dpi, 0, -1), (size, size));
        }
        assert_eq!(icon_dimensions(144, 19, 21), (19, 21));
        assert_eq!(icon_dimensions(u32::MAX, 0, 1024), (256, 256));
    }

    #[test]
    fn unregistered_icon_resizes_without_shell_notifications() {
        let mut tray = fake_tray();
        tray.refresh_icon_with(
            (20, 20),
            |size| {
                assert_eq!(size, (20, 20));
                Some(OwnedIcon(HICON::default()))
            },
            |_, _| panic!("an unregistered icon must not be modified"),
        );
        assert_eq!(tray.icon_size, (20, 20));
        assert!(tray.icon.is_some());
        assert!(!tray.added);
    }

    #[test]
    fn registered_icon_replacement_retries_without_losing_the_old_icon() {
        let mut tray = fake_tray();
        tray.icon = Some(OwnedIcon(HICON::default()));
        tray.added = true;
        let mut messages = Vec::new();
        for succeeds in [false, true] {
            tray.refresh_icon_with(
                (32, 32),
                |_| Some(OwnedIcon(HICON::default())),
                |message, data| {
                    messages.push(message);
                    assert_eq!(data.uFlags, NIF_ICON);
                    assert_eq!(data.uID, 1);
                    succeeds
                },
            );
            assert!(tray.added);
            assert!(tray.icon.is_some());
            assert_eq!(tray.icon_size, if succeeds { (32, 32) } else { (16, 16) });
        }
        assert_eq!(messages, [NIM_MODIFY, NIM_MODIFY]);
        tray.refresh_icon_with(
            (32, 32),
            |_| panic!("same-size icon must be reused"),
            |_, _| panic!("same-size icon must not notify the Shell"),
        );
    }

    #[test]
    fn failed_icon_load_preserves_previous_dimensions_and_registration() {
        let mut tray = fake_tray();
        tray.icon = Some(OwnedIcon(HICON::default()));
        tray.added = true;
        tray.refresh_icon_with(
            (24, 24),
            |_| None,
            |_, _| panic!("a failed load must not replace the registered icon"),
        );
        assert_eq!(tray.icon_size, (16, 16));
        assert!(tray.icon.is_some());
        assert!(tray.added);
    }

    #[test]
    fn owned_icon_shutdown_is_idempotent_without_shell_registration() {
        let mut tray = fake_tray();
        // A null synthetic handle is never passed to DestroyIcon by OwnedIcon.
        tray.icon = Some(OwnedIcon(HICON::default()));
        tray.last_add_attempt = Some(Instant::now());
        tray.shutdown();
        tray.shutdown();
        assert!(tray.icon.is_none());
        assert!(tray.last_add_attempt.is_none());
        assert!(!tray.added);
    }

    fn tip_text(data: &NOTIFYICONDATAW) -> String {
        let len = data.szTip.iter().position(|&c| c == 0).unwrap();
        String::from_utf16(&data.szTip[..len]).unwrap()
    }

    #[test]
    fn successful_add_registers_v4_tooltip_and_skips_duplicates() {
        let mut tray = fake_tray();
        let now = Instant::now();
        let mut messages = Vec::new();
        tray.ensure_added_with(now, |message, data| {
            messages.push(message);
            assert_eq!(data.cbSize, std::mem::size_of::<NOTIFYICONDATAW>() as u32);
            assert_eq!(data.hWnd, HWND::default());
            assert_eq!(data.uID, 1);
            assert_eq!(data.uCallbackMessage, crate::TRAY_CALLBACK_MSG);
            assert_eq!(data.uFlags, NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
            assert_eq!(tip_text(data), "NetSpeed");
            if message == NIM_SETVERSION {
                assert_eq!(unsafe { data.Anonymous.uVersion }, NOTIFYICON_VERSION_4);
            }
            true
        });
        assert_eq!(messages, [NIM_ADD, NIM_SETVERSION]);
        assert!(tray.added);
        assert!(tray.last_add_attempt.is_none());
        tray.ensure_added_with(now + ADD_RETRY_INTERVAL, |_, _| {
            panic!("an already-added icon must not be registered twice")
        });
    }

    #[test]
    fn failed_add_retries_only_after_each_full_retry_interval() {
        let mut tray = fake_tray();
        let now = Instant::now();
        let retry_at = now + ADD_RETRY_INTERVAL;
        let next_retry_at = retry_at + ADD_RETRY_INTERVAL;
        let mut messages = Vec::new();
        tray.ensure_added_with(now, |message, _| {
            messages.push(message);
            false
        });
        assert!(!tray.added);
        assert_eq!(tray.last_add_attempt, Some(now));
        assert_eq!(messages, [NIM_ADD]);
        for early in [now, retry_at - Duration::from_nanos(1)] {
            tray.ensure_added_with(early, |_, _| panic!("retry cooldown must be respected"));
        }
        tray.ensure_added_with(retry_at, |message, _| {
            messages.push(message);
            false
        });
        assert!(!tray.added);
        assert_eq!(tray.last_add_attempt, Some(retry_at));
        assert_eq!(messages, [NIM_ADD, NIM_ADD]);
        tray.ensure_added_with(next_retry_at - Duration::from_nanos(1), |_, _| {
            panic!("each failed add must restart the cooldown")
        });
        tray.ensure_added_with(next_retry_at, |message, _| {
            messages.push(message);
            true
        });
        assert_eq!(messages, [NIM_ADD, NIM_ADD, NIM_ADD, NIM_SETVERSION]);
        assert!(tray.added);
        assert!(tray.last_add_attempt.is_none());
    }

    #[test]
    fn version_failure_does_not_undo_successful_registration() {
        let mut tray = fake_tray();
        let now = Instant::now();
        let mut messages = Vec::new();
        tray.ensure_added_with(now, |message, _| {
            messages.push(message);
            message == NIM_ADD
        });
        assert_eq!(messages, [NIM_ADD, NIM_SETVERSION]);
        assert!(tray.added);
        tray.ensure_added_with(now + ADD_RETRY_INTERVAL, |_, _| {
            panic!("version failure must not cause a duplicate NIM_ADD")
        });
    }

    #[test]
    fn taskbar_restart_forces_add_despite_registration_or_recent_failure() {
        let now = Instant::now();
        for already_added in [false, true] {
            let mut tray = fake_tray();
            tray.added = already_added;
            if !already_added {
                tray.last_add_attempt = Some(now);
            }
            let mut messages = Vec::new();
            tray.re_add_with(now + Duration::from_millis(1), |message, _| {
                messages.push(message);
                true
            });
            assert_eq!(messages, [NIM_ADD, NIM_SETVERSION]);
            assert!(tray.added);
            assert!(tray.last_add_attempt.is_none());
        }
    }

    #[test]
    fn failed_taskbar_readd_remains_retryable_without_version_negotiation() {
        let mut tray = fake_tray();
        tray.added = true;
        let now = Instant::now();
        let mut messages = Vec::new();
        tray.re_add_with(now, |message, _| {
            messages.push(message);
            false
        });
        assert_eq!(messages, [NIM_ADD]);
        assert!(!tray.added);
        assert_eq!(tray.last_add_attempt, Some(now));
        tray.ensure_added_with(now, |_, _| panic!("immediate tick must not retry again"));
        tray.ensure_added_with(now + ADD_RETRY_INTERVAL, |message, _| {
            messages.push(message);
            true
        });
        assert_eq!(messages, [NIM_ADD, NIM_ADD, NIM_SETVERSION]);
        assert!(tray.added);
    }

    #[test]
    fn tooltip_updates_show_standard_tip_only_for_registered_icons() {
        let mut tray = fake_tray();
        tray.update_tip_with("Down: 1.2 MB/s", |_, _| {
            panic!("unregistered icons must not receive NIM_MODIFY")
        });
        tray.added = true;
        let mut calls = 0;
        tray.update_tip_with("Down: 1.2 MB/s", |message, data| {
            calls += 1;
            assert_eq!(message, NIM_MODIFY);
            assert_eq!(data.uFlags, NIF_TIP | NIF_SHOWTIP);
            assert_eq!(tip_text(data), "Down: 1.2 MB/s");
            true
        });
        assert_eq!(calls, 1);
        assert!(tray.added);
    }
}
