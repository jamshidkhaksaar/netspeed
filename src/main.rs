#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(unsafe_op_in_unsafe_fn)]

mod anchor;
mod app;
mod config;
mod format;
mod log;
mod net;
mod overlay;
mod theme;
mod tray;
mod util;

use std::sync::atomic::{AtomicPtr, Ordering};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HICON,
    LoadCursorW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, TranslateMessage, MSG,
    WNDCLASSW, WNDCLASS_STYLES, WS_EX_TOOLWINDOW, WS_POPUP, IDC_ARROW, WM_CONTEXTMENU, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_LBUTTONUP, WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_THEMECHANGED, WM_TIMER,
};

pub static APP: AtomicPtr<app::App> = AtomicPtr::new(std::ptr::null_mut());
pub const TIMER_ID: usize = 1;
pub const TRAY_CALLBACK_MSG: u32 = 0x8001;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--debug-anchor") {
        anchor::debug_dump();
        return;
    }
    if args.iter().any(|a| a == "--debug-net") {
        net_debug();
        return;
    }
    let snap_path = args
        .iter()
        .position(|a| a == "--snap")
        .and_then(|i| args.get(i + 1).cloned());
    unsafe {
        let mutex_name = util::wide("Global\\NetSpeed_2026");
        let mutex = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()));
        if mutex.is_err() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let hmodule = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let inst = HINSTANCE(hmodule.0);
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let wc_tray = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: HICON::default(),
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: w!("NetSpeedTray"),
        };
        let atom_tray = RegisterClassW(&wc_tray);
        let wc_overlay = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: HICON::default(),
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: overlay::CLASS,
        };
        let atom_overlay = RegisterClassW(&wc_overlay);
        if atom_tray == 0 || atom_overlay == 0 {
            util::log_error(&format!(
                "RegisterClassW failed (tray={atom_tray}, overlay={atom_overlay})"
            ));
        }

        let tray_hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("NetSpeedTray"),
            w!("NetSpeed"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(inst),
            None,
        )
        .unwrap_or_default();

        let tbc = RegisterWindowMessageW(w!("TaskbarCreated"));
        let cfg = config::Config::load();
        let mut app_box = Box::new(app::App::new(cfg, tray_hwnd, tbc));
        app_box.snap_path = snap_path;
        app_box.init();
        app_box.start_timer();
        APP.store(Box::into_raw(app_box), Ordering::SeqCst);

        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let ptr = APP.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !ptr.is_null() {
            let mut app = Box::from_raw(ptr);
            app.shutdown();
        }
        let _ = DestroyWindow(tray_hwnd);
    }
}

fn app_ref<'a>() -> Option<&'a mut app::App> {
    let p = APP.load(Ordering::SeqCst);
    if p.is_null() {
        None
    } else {
        Some(unsafe { &mut *p })
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(a) = app_ref() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    if a.tray_hwnd() != hwnd {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_TIMER => {
            a.tick();
            LRESULT(0)
        }
        TRAY_CALLBACK_MSG => {
            let event = (lparam.0 as u32) & 0xFFFF;
            match event {
                WM_CONTEXTMENU | WM_RBUTTONUP => {
                    let cmd = a.show_menu();
                    if cmd != 0 && !a.on_menu_command(cmd) {
                        a.shutdown();
                        PostQuitMessage(0);
                    }
                    LRESULT(0)
                }
                WM_LBUTTONUP | NIN_SELECT => {
                    a.toggle_show();
                    LRESULT(0)
                }
                _ => LRESULT(0),
            }
        }
        m if m == a.taskbar_created_msg() && m != 0 => {
            a.on_taskbar_created();
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_THEMECHANGED => {
            a.on_display_change();
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            a.on_resume();
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn net_debug() {
    let rows = net::list_adapters();
    println!("adapters: {}", rows.len());
    for r in &rows {
        println!(
            "  luid={:>5} idx={:>3} up={} type={:<3} link={:>12} alias={:24} desc={}",
            r.luid, r.index, r.up, r.if_type, r.link_bps, r.alias, r.description
        );
    }
    println!("best route index: {:?}", net::best_route_index());
    let mut prev: Option<(u64, u64, std::time::Instant)> = None;
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let rows = net::list_adapters();
        let best = net::best_route_index()
            .and_then(|i| rows.iter().find(|r| r.index == i).cloned())
            .or_else(|| {
                rows.iter()
                    .filter(|r| r.up && r.is_physical_like())
                    .max_by_key(|r| r.link_bps)
                    .cloned()
            });
        if let Some(r) = best {
            let now = std::time::Instant::now();
            if let Some((pi, po, pt)) = prev {
                let dt = now.duration_since(pt).as_secs_f64();
                if dt > 0.0 && r.in_octets >= pi && r.out_octets >= po {
                    println!(
                        "  {}: down {} up {}",
                        r.alias,
                        format::format_rate(
                            (r.in_octets - pi) as f64 / dt,
                            config::Units::BytesBin,
                            1
                        ),
                        format::format_rate(
                            (r.out_octets - po) as f64 / dt,
                            config::Units::BytesBin,
                            1
                        )
                    );
                }
            }
            prev = Some((r.in_octets, r.out_octets, now));
        }
    }
}
